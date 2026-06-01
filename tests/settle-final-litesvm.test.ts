import * as anchor from "@coral-xyz/anchor";
import { BN, Program } from "@coral-xyz/anchor";
import { Keypair, PublicKey, SystemProgram, AccountMeta } from "@solana/web3.js";
import { TOKEN_PROGRAM_ID } from "@solana/spl-token";
import { expect } from "chai";

import { BracketChain } from "../target/types/bracket_chain";
import {
  ENTRY_FEE,
  buildBracketDescriptors,
  createUsdcLikeMint,
  findMatchPda,
  findParticipantPda,
  findProtocolConfigPda,
  findTournamentPda,
  findVaultPda,
  makeAtaOnly,
  makeFundedWallet,
  sendStartChunks,
  tokenBalance,
} from "./utils";

// ─────────────────────────────────────────────────────────────────────────────
// H-1 — multi-placement final hardening (`settle_final` + the finalize gate).
//
// On single-elim there is no 3rd-place match, so `placements[2..]` are
// unconstrained on-chain (only [0]/[1] are validated in `distribute_prizes`).
// The fix (Крок 0) rejects a multi-placement (non-WinnerTakesAll) final on every
// PERMISSIONLESS / COUNTERPARTY path (`claim_result` / `force_claim_disputed` /
// `confirm_result`), and (Variant B) adds the trusted `settle_final`
// (arbitrator-signed; winner pinned to the proposal) as the only path that may
// adjudicate placements 3..N. WinnerTakesAll finals stay permissionlessly
// claimable, proving the happy path is intact.
//
// 4-player Standard (60/25/15) bracket, `Manual` game, `PlayerReported` mode,
// SlotHashes-seeded, dispute_window_secs = 0 (so finalize is immediately
// eligible). Mirrors the conventions in `player-reported.test.ts`.
// ─────────────────────────────────────────────────────────────────────────────
describe("settle_final / H-1 multi-placement final hardening", function () {
  this.timeout(300_000);

  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);
  const program = anchor.workspace.bracketChain as Program<BracketChain>;
  const programId = program.programId;

  let usdcMint: PublicKey;
  let treasuryAta: PublicKey;
  let protocolConfigPda: PublicKey;

  before(async () => {
    usdcMint = await createUsdcLikeMint(provider);
    [protocolConfigPda] = findProtocolConfigPda(programId);

    const existing = await program.account.protocolConfig.fetchNullable(protocolConfigPda);
    if (!existing) {
      const treasury = Keypair.generate().publicKey;
      await program.methods
        .initializeProtocol()
        .accountsPartial({
          authority: provider.wallet.publicKey,
          protocolConfig: protocolConfigPda,
          treasury,
          defaultMint: usdcMint,
          systemProgram: SystemProgram.programId,
        })
        .rpc();
      treasuryAta = await makeAtaOnly(provider, usdcMint, treasury);
    } else {
      treasuryAta = await makeAtaOnly(provider, usdcMint, existing.treasury);
    }
  });

  // create + join 4 players, then start (SlotHash seed). `preset` selects the
  // payout structure under test.
  async function setupStartedBracket(opts: {
    name: string;
    preset: any;
  }) {
    const organizer = (await makeFundedWallet(provider, usdcMint, new BN(0), 0.2)).keypair;
    const [tournamentPda] = findTournamentPda(organizer.publicKey, opts.name, programId);
    const [vaultPda] = findVaultPda(tournamentPda, programId);
    const deadline = new BN(Math.floor(Date.now() / 1000) + 3600);

    await program.methods
      .createTournament(
        opts.name,
        ENTRY_FEE,
        4,
        opts.preset,
        deadline,
        new BN(0),
        { manual: {} },
        { playerReported: {} },
        0, // dispute_window_secs = 0 → finalize immediately eligible
      )
      .accountsPartial({
        organizer: organizer.publicKey,
        protocolConfig: protocolConfigPda,
        tokenMint: usdcMint,
        tournament: tournamentPda,
        vault: vaultPda,
        organizerTokenAccount: null,
        tokenProgram: TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
        rent: anchor.web3.SYSVAR_RENT_PUBKEY,
      })
      .signers([organizer])
      .rpc();

    const players: { keypair: Keypair; ata: PublicKey; participantPda: PublicKey }[] = [];
    for (let i = 0; i < 4; i++) {
      const w = await makeFundedWallet(provider, usdcMint, ENTRY_FEE);
      const [participantPda] = findParticipantPda(tournamentPda, w.keypair.publicKey, programId);
      await program.methods
        .joinTournament()
        .accountsPartial({
          player: w.keypair.publicKey,
          tournament: tournamentPda,
          protocolConfig: protocolConfigPda,
          participant: participantPda,
          playerTokenAccount: w.ata,
          vault: vaultPda,
          gameIdentityAttestation: null,
          tokenProgram: TOKEN_PROGRAM_ID,
          systemProgram: SystemProgram.programId,
        })
        .signers([w.keypair])
        .rpc();
      players.push({ keypair: w.keypair, ata: w.ata, participantPda });
    }

    const playerKeys = players.map((p) => p.keypair.publicKey);
    const { descriptors, matchPdas } = buildBracketDescriptors(tournamentPda, playerKeys, programId);
    await sendStartChunks(program, organizer, tournamentPda, descriptors, matchPdas);

    return { organizer, tournamentPda, vaultPda, players };
  }

  async function finalizeAccounts(tournamentPda: PublicKey, matchPda: PublicKey) {
    const m = await program.account.matchNode.fetch(matchPda);
    const [participantA] = findParticipantPda(tournamentPda, m.playerA, programId);
    const [participantB] = findParticipantPda(tournamentPda, m.playerB, programId);
    return { participantA, participantB, playerA: m.playerA, playerB: m.playerB };
  }

  function keypairFor(players: { keypair: Keypair }[], pk: PublicKey): Keypair {
    return players.find((p) => p.keypair.publicKey.equals(pk))!.keypair;
  }
  function ataFor(players: { keypair: Keypair; ata: PublicKey }[], pk: PublicKey): PublicKey {
    return players.find((p) => p.keypair.publicKey.equals(pk))!.ata;
  }

  async function expectError(p: Promise<unknown>, code: string) {
    try {
      await p;
      throw new Error(`expected error ${code}, but call succeeded`);
    } catch (e: any) {
      const got = e?.error?.errorCode?.code ?? e?.message ?? String(e);
      expect(got, `expected ${code}, got: ${got}`).to.contain(code);
    }
  }

  // Advance a semi by propose(self) + permissionless claim (window=0). Non-final
  // claims carry no placements, so the gate does not apply to them.
  async function advanceSemiByClaim(
    tournamentPda: PublicKey,
    vaultPda: PublicKey,
    players: { keypair: Keypair }[],
    semiPda: PublicKey,
    finalPda: PublicKey,
    cranker: Keypair,
  ) {
    const s = await finalizeAccounts(tournamentPda, semiPda);
    await program.methods
      .proposeResult(s.playerA)
      .accountsPartial({ proposer: s.playerA, tournament: tournamentPda, matchAccount: semiPda })
      .signers([keypairFor(players, s.playerA)])
      .rpc();
    await program.methods
      .claimResult([])
      .accountsPartial({
        payer: cranker.publicKey,
        tournament: tournamentPda,
        matchAccount: semiPda,
        nextMatch: finalPda,
        participantA: s.participantA,
        participantB: s.participantB,
        protocolConfig: protocolConfigPda,
        vault: vaultPda,
        organizerTokenAccount: null,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .signers([cranker])
      .rpc();
    return s; // playerA = winner (advanced), playerB = semifinal loser
  }

  it("blocks permissionless multi-placement final, then settles it via the arbitrator", async () => {
    const { organizer, tournamentPda, vaultPda, players } = await setupStartedBracket({
      name: "h1-standard",
      preset: { standard: {} },
    });

    const [semi0] = findMatchPda(tournamentPda, 0, 0, programId);
    const [semi1] = findMatchPda(tournamentPda, 0, 1, programId);
    const [finalPda] = findMatchPda(tournamentPda, 1, 0, programId);
    const cranker = (await makeFundedWallet(provider, usdcMint, new BN(0), 0.05)).keypair;

    // Advance both semis; their losers are the two 3rd/4th-place candidates.
    const s0 = await advanceSemiByClaim(tournamentPda, vaultPda, players, semi0, finalPda, cranker);
    const s1 = await advanceSemiByClaim(tournamentPda, vaultPda, players, semi1, finalPda, cranker);

    const f = await finalizeAccounts(tournamentPda, finalPda);
    const champion = f.playerA; // winner we will propose
    const runnerUp = f.playerB;
    const semifinalLosers = [s0.playerB, s1.playerB];
    const thirdPlace = semifinalLosers[0]; // arbitrator's adjudication

    // Propose the final's winner (a player in the match). Window=0 → eligible now.
    await program.methods
      .proposeResult(champion)
      .accountsPartial({ proposer: champion, tournament: tournamentPda, matchAccount: finalPda })
      .signers([keypairFor(players, champion)])
      .rpc();

    const finalize = {
      tournament: tournamentPda,
      matchAccount: finalPda,
      nextMatch: null,
      participantA: f.participantA,
      participantB: f.participantB,
      protocolConfig: protocolConfigPda,
      vault: vaultPda,
      organizerTokenAccount: null,
      tokenProgram: TOKEN_PROGRAM_ID,
    };

    // ── NEG 1: the theft vector — a permissionless cranker tries to claim the
    // Standard final and redirect 3rd place to an attacker wallet. Rejected by
    // the finalize gate before any transfer. ──
    const attacker = (await makeFundedWallet(provider, usdcMint, new BN(0), 0.05)).keypair;
    const attackerAta = await makeAtaOnly(provider, usdcMint, attacker.publicKey);
    const stolenRemaining: AccountMeta[] = [
      { pubkey: ataFor(players, champion), isSigner: false, isWritable: true },
      { pubkey: ataFor(players, runnerUp), isSigner: false, isWritable: true },
      { pubkey: attackerAta, isSigner: false, isWritable: true },
      { pubkey: treasuryAta, isSigner: false, isWritable: true },
    ];
    await expectError(
      program.methods
        .claimResult([champion, runnerUp, attacker.publicKey])
        .accountsPartial({ payer: cranker.publicKey, ...finalize })
        .remainingAccounts(stolenRemaining)
        .signers([cranker])
        .rpc(),
      "UntrustedMultiPlacementFinal",
    );

    // ── NEG 2: a non-arbitrator cannot settle_final (address constraint). ──
    const correctRemaining: AccountMeta[] = [
      { pubkey: ataFor(players, champion), isSigner: false, isWritable: true },
      { pubkey: ataFor(players, runnerUp), isSigner: false, isWritable: true },
      { pubkey: ataFor(players, thirdPlace), isSigner: false, isWritable: true },
      { pubkey: treasuryAta, isSigner: false, isWritable: true },
    ];
    await expectError(
      program.methods
        .settleFinal([champion, runnerUp, thirdPlace])
        .accountsPartial({ arbitrator: attacker.publicKey, ...finalize })
        .remainingAccounts(correctRemaining)
        .signers([attacker])
        .rpc(),
      "UnauthorizedAuthority",
    );

    // ── POS: the arbitrator (defaults to organizer) settles with adjudicated
    // placements. Standard 60/25/15 over the net pool; 3.5% fee to treasury. ──
    const treasuryBefore = await tokenBalance(provider.connection, treasuryAta);
    await program.methods
      .settleFinal([champion, runnerUp, thirdPlace])
      .accountsPartial({ arbitrator: organizer.publicKey, ...finalize })
      .remainingAccounts(correctRemaining)
      .signers([organizer])
      .rpc();

    const grossPool = 4n * BigInt(ENTRY_FEE.toString());
    const fee = (grossPool * 350n) / 10_000n;
    const net = grossPool - fee;
    const expectChampion = (net * 6000n) / 10_000n;
    const expectRunner = (net * 2500n) / 10_000n;
    const expectThird = (net * 1500n) / 10_000n;

    expect(await tokenBalance(provider.connection, ataFor(players, champion))).to.equal(expectChampion);
    expect(await tokenBalance(provider.connection, ataFor(players, runnerUp))).to.equal(expectRunner);
    expect(await tokenBalance(provider.connection, ataFor(players, thirdPlace))).to.equal(expectThird);
    expect((await tokenBalance(provider.connection, treasuryAta)) - treasuryBefore).to.equal(fee);

    const t = await program.account.tournament.fetch(tournamentPda);
    expect(t.status).to.deep.equal({ completed: {} });
    expect(t.champion.toBase58()).to.equal(champion.toBase58());
    const fin = await program.account.matchNode.fetch(finalPda);
    expect(fin.status).to.deep.equal({ completed: {} });
  });

  it("still lets anyone permissionlessly claim a WinnerTakesAll final (happy path intact)", async () => {
    const { tournamentPda, vaultPda, players } = await setupStartedBracket({
      name: "h1-wta",
      preset: { winnerTakesAll: {} },
    });

    const [semi0] = findMatchPda(tournamentPda, 0, 0, programId);
    const [semi1] = findMatchPda(tournamentPda, 0, 1, programId);
    const [finalPda] = findMatchPda(tournamentPda, 1, 0, programId);
    const cranker = (await makeFundedWallet(provider, usdcMint, new BN(0), 0.05)).keypair;

    await advanceSemiByClaim(tournamentPda, vaultPda, players, semi0, finalPda, cranker);
    await advanceSemiByClaim(tournamentPda, vaultPda, players, semi1, finalPda, cranker);

    const f = await finalizeAccounts(tournamentPda, finalPda);
    const champion = f.playerA;

    await program.methods
      .proposeResult(champion)
      .accountsPartial({ proposer: champion, tournament: tournamentPda, matchAccount: finalPda })
      .signers([keypairFor(players, champion)])
      .rpc();

    const remaining: AccountMeta[] = [
      { pubkey: ataFor(players, champion), isSigner: false, isWritable: true },
      { pubkey: treasuryAta, isSigner: false, isWritable: true },
    ];
    // placement_count == 1 → the gate allows the permissionless path.
    await program.methods
      .claimResult([champion])
      .accountsPartial({
        payer: cranker.publicKey,
        tournament: tournamentPda,
        matchAccount: finalPda,
        nextMatch: null,
        participantA: f.participantA,
        participantB: f.participantB,
        protocolConfig: protocolConfigPda,
        vault: vaultPda,
        organizerTokenAccount: null,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .remainingAccounts(remaining)
      .signers([cranker])
      .rpc();

    const grossPool = 4n * BigInt(ENTRY_FEE.toString());
    const net = grossPool - (grossPool * 350n) / 10_000n;
    expect(await tokenBalance(provider.connection, ataFor(players, champion))).to.equal(net);

    const t = await program.account.tournament.fetch(tournamentPda);
    expect(t.status).to.deep.equal({ completed: {} });
  });
});
