import * as anchor from "@coral-xyz/anchor";
import { BN, Program } from "@coral-xyz/anchor";
import {
  Keypair,
  PublicKey,
  SystemProgram,
  AccountMeta,
} from "@solana/web3.js";
import { TOKEN_PROGRAM_ID } from "@solana/spl-token";
import { expect } from "chai";

import { BracketChain } from "../target/types/bracket_chain";
import {
  ENTRY_FEE,
  buildBracketDescriptors,
  createUsdcLikeMint,
  ensureProtocolInitialized,
  findMatchPda,
  findParticipantPda,
  findTournamentPda,
  findVaultPda,
  makeFundedWallet,
  sendStartChunks,
  tokenBalance,
} from "./utils";

// ─────────────────────────────────────────────────────────────────────────────
// `organizer_deposit` lifecycle coverage.
// Three tests:
//   1. Pre-start cancel refunds the deposit back to the organizer ATA.
//   2. Calling `cancel_tournament` twice does not double-refund the deposit
//      (idempotency guard via `organizer_deposit_refunded`).
//   3. Variant B on `report_result` final-match: the deposit stays in the
//      vault and is included in the prize-pool basis (payouts + protocol
//      fee apply to `vault.amount`, including the deposit). NOT refunded.
// ─────────────────────────────────────────────────────────────────────────────

const ORGANIZER_DEPOSIT = new BN(5_000_000); // 5 USDC

describe("organizer-deposit", function () {
  this.timeout(300_000);

  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);
  const program = anchor.workspace.bracketChain as Program<BracketChain>;
  const programId = program.programId;

  let usdcMint: PublicKey;
  let protocolConfigPda: PublicKey;
  let treasuryAta: PublicKey;

  before(async () => {
    usdcMint = await createUsdcLikeMint(provider);
    const init = await ensureProtocolInitialized(provider, program, usdcMint);
    protocolConfigPda = init.protocolConfigPda;
    treasuryAta = init.treasuryAta;
  });

  async function createWithDeposit(opts: {
    name: string;
    playerCount: number;
    maxParticipants: number;
    payoutPreset: any;
    deposit: BN;
  }) {
    const organizer = await makeFundedWallet(provider, usdcMint, opts.deposit, 0.2);
    const [tournamentPda] = findTournamentPda(organizer.keypair.publicKey, opts.name, programId);
    const [vaultPda] = findVaultPda(tournamentPda, programId);

    const deadline = new BN(Math.floor(Date.now() / 1000) + 3600);
    await program.methods
      .createTournament(
        opts.name,
        ENTRY_FEE,
        opts.maxParticipants,
        opts.payoutPreset,
        deadline,
        opts.deposit,
      )
      .accountsPartial({
        organizer: organizer.keypair.publicKey,
        protocolConfig: protocolConfigPda,
        tokenMint: usdcMint,
        tournament: tournamentPda,
        vault: vaultPda,
        organizerTokenAccount: organizer.ata,
        tokenProgram: TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
        rent: anchor.web3.SYSVAR_RENT_PUBKEY,
      })
      .signers([organizer.keypair])
      .rpc();

    const players: { keypair: Keypair; ata: PublicKey; participantPda: PublicKey }[] = [];
    for (let i = 0; i < opts.playerCount; i++) {
      const w = await makeFundedWallet(provider, usdcMint, ENTRY_FEE);
      const [participantPda] = findParticipantPda(tournamentPda, w.keypair.publicKey, programId);
      await program.methods
        .joinTournament()
        .accountsPartial({
          player: w.keypair.publicKey,
          tournament: tournamentPda,
          participant: participantPda,
          playerTokenAccount: w.ata,
          vault: vaultPda,
          tokenProgram: TOKEN_PROGRAM_ID,
          systemProgram: SystemProgram.programId,
        })
        .signers([w.keypair])
        .rpc();
      players.push({ keypair: w.keypair, ata: w.ata, participantPda });
    }

    return { organizer, tournamentPda, vaultPda, players };
  }

  // ───────────────────────────────────────────────────────────────────────────
  // 1. Pre-start cancel refunds the deposit
  // ───────────────────────────────────────────────────────────────────────────
  it("refunds organizer_deposit back to organizer ATA on pre-start cancel", async () => {
    const { organizer, tournamentPda, vaultPda, players } = await createWithDeposit({
      name: "od-cancel-1",
      playerCount: 2,
      maxParticipants: 4,
      payoutPreset: { winnerTakesAll: {} },
      deposit: ORGANIZER_DEPOSIT,
    });

    // Deposit transferred into vault at create — organizer ATA should be 0.
    expect(await tokenBalance(provider.connection, organizer.ata)).to.equal(0n);

    const remaining: AccountMeta[] = [];
    for (const p of players) {
      remaining.push({ pubkey: p.participantPda, isSigner: false, isWritable: true });
      remaining.push({ pubkey: p.ata, isSigner: false, isWritable: true });
    }

    await program.methods
      .cancelTournament()
      .accountsPartial({
        caller: organizer.keypair.publicKey,
        tournament: tournamentPda,
        vault: vaultPda,
        organizerTokenAccount: organizer.ata,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .remainingAccounts(remaining)
      .signers([organizer.keypair])
      .rpc();

    expect(await tokenBalance(provider.connection, organizer.ata)).to.equal(
      BigInt(ORGANIZER_DEPOSIT.toString()),
    );
    for (const p of players) {
      expect(await tokenBalance(provider.connection, p.ata)).to.equal(BigInt(ENTRY_FEE.toString()));
    }
    expect(await tokenBalance(provider.connection, vaultPda)).to.equal(0n);

    const t = await program.account.tournament.fetch(tournamentPda);
    expect(t.status).to.deep.equal({ cancelled: {} });
    expect(t.organizerDepositRefunded).to.equal(true);
  });

  // ───────────────────────────────────────────────────────────────────────────
  // 2. Idempotency: second cancel call does not double-refund the deposit
  // ───────────────────────────────────────────────────────────────────────────
  it("does not double-refund deposit when cancel_tournament is called twice", async () => {
    const { organizer, tournamentPda, vaultPda, players } = await createWithDeposit({
      name: "od-idemp-2",
      playerCount: 2,
      maxParticipants: 4,
      payoutPreset: { winnerTakesAll: {} },
      deposit: ORGANIZER_DEPOSIT,
    });

    const remaining: AccountMeta[] = [];
    for (const p of players) {
      remaining.push({ pubkey: p.participantPda, isSigner: false, isWritable: true });
      remaining.push({ pubkey: p.ata, isSigner: false, isWritable: true });
    }

    await program.methods
      .cancelTournament()
      .accountsPartial({
        caller: organizer.keypair.publicKey,
        tournament: tournamentPda,
        vault: vaultPda,
        organizerTokenAccount: organizer.ata,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .remainingAccounts(remaining)
      .signers([organizer.keypair])
      .rpc();

    const balanceAfterFirst = await tokenBalance(provider.connection, organizer.ata);
    expect(balanceAfterFirst).to.equal(BigInt(ORGANIZER_DEPOSIT.toString()));

    // Second call — any signer can drive permissionless chunks once Cancelled.
    // Empty remaining_accounts (no participants left) + organizer ATA still
    // attached, to exercise the deposit-refund guard explicitly.
    const secondCaller = await makeFundedWallet(provider, usdcMint, new BN(0), 0.05);
    await program.methods
      .cancelTournament()
      .accountsPartial({
        caller: secondCaller.keypair.publicKey,
        tournament: tournamentPda,
        vault: vaultPda,
        organizerTokenAccount: organizer.ata,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .remainingAccounts([])
      .signers([secondCaller.keypair])
      .rpc();

    expect(await tokenBalance(provider.connection, organizer.ata)).to.equal(balanceAfterFirst);
    expect(await tokenBalance(provider.connection, vaultPda)).to.equal(0n);

    const t = await program.account.tournament.fetch(tournamentPda);
    expect(t.organizerDepositRefunded).to.equal(true);
  });

  // ───────────────────────────────────────────────────────────────────────────
  // 3. Variant B: deposit included in prize-pool basis on final-match
  // ───────────────────────────────────────────────────────────────────────────
  it("includes organizer_deposit in prize-pool basis and does not refund it on final-match", async () => {
    const { organizer, tournamentPda, vaultPda, players } = await createWithDeposit({
      name: "od-final-3",
      playerCount: 4,
      maxParticipants: 4,
      payoutPreset: { winnerTakesAll: {} },
      deposit: ORGANIZER_DEPOSIT,
    });

    const playerKeys = players.map((p) => p.keypair.publicKey);
    const { descriptors, matchPdas } = buildBracketDescriptors(tournamentPda, playerKeys, programId);
    await sendStartChunks(program, organizer.keypair, tournamentPda, descriptors, matchPdas);

    const treasuryBefore = await tokenBalance(provider.connection, treasuryAta);
    const organizerBefore = await tokenBalance(provider.connection, organizer.ata);

    // Round 0: players[0] beats players[1] → advances to final;
    //           players[2] beats players[3] → advances to final.
    const [r0m0] = findMatchPda(tournamentPda, 0, 0, programId);
    const [r0m1] = findMatchPda(tournamentPda, 0, 1, programId);
    const [finalMatch] = findMatchPda(tournamentPda, 1, 0, programId);

    await program.methods
      .reportResult(playerKeys[0], [])
      .accountsPartial({
        organizer: organizer.keypair.publicKey,
        tournament: tournamentPda,
        matchAccount: r0m0,
        nextMatch: finalMatch,
        protocolConfig: protocolConfigPda,
        vault: vaultPda,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .signers([organizer.keypair])
      .rpc();

    await program.methods
      .reportResult(playerKeys[2], [])
      .accountsPartial({
        organizer: organizer.keypair.publicKey,
        tournament: tournamentPda,
        matchAccount: r0m1,
        nextMatch: finalMatch,
        protocolConfig: protocolConfigPda,
        vault: vaultPda,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .signers([organizer.keypair])
      .rpc();

    // Vault before final = 4 * entry_fee + deposit.
    const expectedVaultBeforeFinal =
      4n * BigInt(ENTRY_FEE.toString()) + BigInt(ORGANIZER_DEPOSIT.toString());
    expect(await tokenBalance(provider.connection, vaultPda)).to.equal(expectedVaultBeforeFinal);

    // Final.
    const remaining: AccountMeta[] = [
      { pubkey: players[0].ata, isSigner: false, isWritable: true },
      { pubkey: treasuryAta, isSigner: false, isWritable: true },
    ];

    await program.methods
      .reportResult(playerKeys[0], [playerKeys[0]])
      .accountsPartial({
        organizer: organizer.keypair.publicKey,
        tournament: tournamentPda,
        matchAccount: finalMatch,
        nextMatch: null,
        protocolConfig: protocolConfigPda,
        vault: vaultPda,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .remainingAccounts(remaining)
      .signers([organizer.keypair])
      .rpc();

    // Variant B: basis = vault.amount = 4*entry_fee + deposit. Fee+payout apply to that.
    const grossBasis =
      4n * BigInt(ENTRY_FEE.toString()) + BigInt(ORGANIZER_DEPOSIT.toString());
    const expectedFee = (grossBasis * 350n) / 10_000n;
    const expectedChampion = grossBasis - expectedFee;

    const treasuryAfter = await tokenBalance(provider.connection, treasuryAta);
    const organizerAfter = await tokenBalance(provider.connection, organizer.ata);
    const championAfter = await tokenBalance(provider.connection, players[0].ata);

    expect(treasuryAfter - treasuryBefore).to.equal(expectedFee);
    // Variant B: deposit stays in the pool — organizer balance unchanged on completion.
    expect(organizerAfter - organizerBefore).to.equal(0n);
    expect(championAfter).to.equal(expectedChampion);
    expect(await tokenBalance(provider.connection, vaultPda)).to.equal(0n);

    const t = await program.account.tournament.fetch(tournamentPda);
    expect(t.status).to.deep.equal({ completed: {} });
    expect(t.champion.toBase58()).to.equal(playerKeys[0].toBase58());
    // Refund flag only flips on the Cancelled path; completion leaves it false.
    expect(t.organizerDepositRefunded).to.equal(false);

    expect(BigInt(t.entryFee.toString())).to.equal(BigInt(ENTRY_FEE.toString()));
  });
});
