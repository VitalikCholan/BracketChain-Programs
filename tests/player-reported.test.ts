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
// Stage B — player-reported settlement (propose / confirm / dispute / claim /
// resolve / force-claim). Uses a 4-player single-elim bracket (2 semis + final)
// in `Manual` game (no SAS) + `PlayerReported` mode, seeded via the SlotHashes
// fallback (no VRF account bound — see DEC-B1). Time-gated paths use
// dispute_window_secs = 0 so `claim_result` is immediately eligible; the 24h
// `force_claim_disputed` happy path is only assert-rejected pre-window here
// (full path is a devnet/manual test — can't fast-forward 24h on a validator).
// ─────────────────────────────────────────────────────────────────────────────
describe("player-reported settlement", function () {
  this.timeout(300_000);

  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);
  const program = anchor.workspace.bracketChain as Program<BracketChain>;
  const programId = program.programId;

  let usdcMint: PublicKey;
  let treasuryWallet: Keypair;
  let treasuryAta: PublicKey;
  let protocolConfigPda: PublicKey;

  before(async () => {
    usdcMint = await createUsdcLikeMint(provider);
    treasuryWallet = Keypair.generate();
    treasuryAta = await makeAtaOnly(provider, usdcMint, treasuryWallet.publicKey);
    [protocolConfigPda] = findProtocolConfigPda(programId);

    // Shared validator across files — initialize only if absent.
    const existing = await program.account.protocolConfig.fetchNullable(protocolConfigPda);
    if (!existing) {
      await program.methods
        .initializeProtocol()
        .accountsPartial({
          authority: provider.wallet.publicKey,
          protocolConfig: protocolConfigPda,
          treasury: treasuryWallet.publicKey,
          defaultMint: usdcMint,
          systemProgram: SystemProgram.programId,
        })
        .rpc();
    } else {
      // Reuse the established treasury so fee transfers validate.
      treasuryWallet = null as any;
      treasuryAta = await makeAtaOnly(provider, usdcMint, existing.treasury);
    }
  });

  // create + join 4 players in PlayerReported mode, then start (SlotHash seed).
  async function setupStartedBracket(opts: {
    name: string;
    settlementMode: any;
    disputeWindowSecs: number;
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
        { winnerTakesAll: {} },
        deadline,
        new BN(0),
        { manual: {} },
        opts.settlementMode,
        opts.disputeWindowSecs,
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

  // Resolve a match's two participant PDAs from on-chain player_a / player_b.
  async function finalizeAccounts(tournamentPda: PublicKey, matchPda: PublicKey) {
    const m = await program.account.matchNode.fetch(matchPda);
    const [participantA] = findParticipantPda(tournamentPda, m.playerA, programId);
    const [participantB] = findParticipantPda(tournamentPda, m.playerB, programId);
    return { participantA, participantB, playerA: m.playerA, playerB: m.playerB };
  }

  function keypairFor(players: { keypair: Keypair }[], pk: PublicKey): Keypair {
    return players.find((p) => p.keypair.publicKey.equals(pk))!.keypair;
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

  // ── 1. propose → confirm: advance + stats, then final distributes prizes ──
  it("propose+confirm advances the bracket, credits stats, and pays the final", async () => {
    const { tournamentPda, vaultPda, players } = await setupStartedBracket({
      name: "pr-happy",
      settlementMode: { playerReported: {} },
      disputeWindowSecs: 60,
    });

    const [semi0] = findMatchPda(tournamentPda, 0, 0, programId);
    const [semi1] = findMatchPda(tournamentPda, 0, 1, programId);
    const [finalPda] = findMatchPda(tournamentPda, 1, 0, programId);
    const [nextFromSemi] = findMatchPda(tournamentPda, 1, 0, programId);

    // Semi 0: playerA proposes self; playerB confirms.
    const s0 = await finalizeAccounts(tournamentPda, semi0);
    await program.methods
      .proposeResult(s0.playerA)
      .accountsPartial({
        proposer: s0.playerA,
        tournament: tournamentPda,
        matchAccount: semi0,
      })
      .signers([keypairFor(players, s0.playerA)])
      .rpc();
    await program.methods
      .confirmResult([])
      .accountsPartial({
        counterparty: s0.playerB,
        tournament: tournamentPda,
        matchAccount: semi0,
        nextMatch: nextFromSemi,
        participantA: s0.participantA,
        participantB: s0.participantB,
        protocolConfig: protocolConfigPda,
        vault: vaultPda,
        organizerTokenAccount: null,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .signers([keypairFor(players, s0.playerB)])
      .rpc();

    // Stats credited.
    const paWin = await program.account.participant.fetch(s0.participantA);
    const pbLose = await program.account.participant.fetch(s0.participantB);
    expect(paWin.wins).to.equal(1);
    expect(pbLose.losses).to.equal(1);

    const semi0Acc = await program.account.matchNode.fetch(semi0);
    expect(semi0Acc.status).to.deep.equal({ completed: {} });
    expect(semi0Acc.winner.toBase58()).to.equal(s0.playerA.toBase58());

    // Semi 1.
    const s1 = await finalizeAccounts(tournamentPda, semi1);
    await program.methods
      .proposeResult(s1.playerA)
      .accountsPartial({ proposer: s1.playerA, tournament: tournamentPda, matchAccount: semi1 })
      .signers([keypairFor(players, s1.playerA)])
      .rpc();
    await program.methods
      .confirmResult([])
      .accountsPartial({
        counterparty: s1.playerB,
        tournament: tournamentPda,
        matchAccount: semi1,
        nextMatch: nextFromSemi,
        participantA: s1.participantA,
        participantB: s1.participantB,
        protocolConfig: protocolConfigPda,
        vault: vaultPda,
        organizerTokenAccount: null,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .signers([keypairFor(players, s1.playerB)])
      .rpc();

    // Final: WTA → champion = final.playerA. Confirm with placements + ATAs.
    const f = await finalizeAccounts(tournamentPda, finalPda);
    const championKp = keypairFor(players, f.playerA);
    const championAta = players.find((p) => p.keypair.publicKey.equals(f.playerA))!.ata;
    const treasuryBefore = await tokenBalance(provider.connection, treasuryAta);

    await program.methods
      .proposeResult(f.playerA)
      .accountsPartial({ proposer: f.playerA, tournament: tournamentPda, matchAccount: finalPda })
      .signers([championKp])
      .rpc();

    const remaining: AccountMeta[] = [
      { pubkey: championAta, isSigner: false, isWritable: true },
      { pubkey: treasuryAta, isSigner: false, isWritable: true },
    ];
    await program.methods
      .confirmResult([f.playerA])
      .accountsPartial({
        counterparty: f.playerB,
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
      .signers([keypairFor(players, f.playerB)])
      .rpc();

    const grossPool = 4n * BigInt(ENTRY_FEE.toString());
    const expectedFee = (grossPool * 350n) / 10_000n;
    const expectedNet = grossPool - expectedFee;
    const treasuryAfter = await tokenBalance(provider.connection, treasuryAta);

    expect(await tokenBalance(provider.connection, championAta)).to.equal(expectedNet);
    expect(treasuryAfter - treasuryBefore).to.equal(expectedFee);
    expect(await tokenBalance(provider.connection, vaultPda)).to.equal(0n);

    const t = await program.account.tournament.fetch(tournamentPda);
    expect(t.status).to.deep.equal({ completed: {} });
    expect(t.champion.toBase58()).to.equal(f.playerA.toBase58());
  });

  // ── 2. propose → dispute → organizer resolves (overrides winner) ──────────
  it("dispute routes to the organizer, who resolves with the override winner", async () => {
    const { organizer, tournamentPda, vaultPda, players } = await setupStartedBracket({
      name: "pr-dispute",
      settlementMode: { playerReported: {} },
      disputeWindowSecs: 60,
    });

    const [semi0] = findMatchPda(tournamentPda, 0, 0, programId);
    const [nextPda] = findMatchPda(tournamentPda, 1, 0, programId);
    const s0 = await finalizeAccounts(tournamentPda, semi0);

    // playerA proposes self; playerB disputes.
    await program.methods
      .proposeResult(s0.playerA)
      .accountsPartial({ proposer: s0.playerA, tournament: tournamentPda, matchAccount: semi0 })
      .signers([keypairFor(players, s0.playerA)])
      .rpc();
    await program.methods
      .disputeResult(1)
      .accountsPartial({ disputer: s0.playerB, tournament: tournamentPda, matchAccount: semi0 })
      .signers([keypairFor(players, s0.playerB)])
      .rpc();

    const disputed = await program.account.matchNode.fetch(semi0);
    expect(disputed.disputed).to.equal(true);
    expect(disputed.disputeReason).to.equal(1);

    // Organizer overrides: playerB wins.
    await program.methods
      .resolveDispute(s0.playerB, [])
      .accountsPartial({
        organizer: organizer.publicKey,
        tournament: tournamentPda,
        matchAccount: semi0,
        nextMatch: nextPda,
        participantA: s0.participantA,
        participantB: s0.participantB,
        protocolConfig: protocolConfigPda,
        vault: vaultPda,
        organizerTokenAccount: null,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .signers([organizer])
      .rpc();

    const resolved = await program.account.matchNode.fetch(semi0);
    expect(resolved.status).to.deep.equal({ completed: {} });
    expect(resolved.winner.toBase58()).to.equal(s0.playerB.toBase58());
    const pbWin = await program.account.participant.fetch(s0.participantB);
    expect(pbWin.wins).to.equal(1);
  });

  // ── 3. propose → permissionless claim after the (zero) dispute window ─────
  it("claim_result lets anyone finalize an undisputed proposal past the window", async () => {
    const { tournamentPda, vaultPda, players } = await setupStartedBracket({
      name: "pr-claim",
      settlementMode: { playerReported: {} },
      disputeWindowSecs: 0,
    });

    const [semi0] = findMatchPda(tournamentPda, 0, 0, programId);
    const [nextPda] = findMatchPda(tournamentPda, 1, 0, programId);
    const s0 = await finalizeAccounts(tournamentPda, semi0);

    await program.methods
      .proposeResult(s0.playerA)
      .accountsPartial({ proposer: s0.playerA, tournament: tournamentPda, matchAccount: semi0 })
      .signers([keypairFor(players, s0.playerA)])
      .rpc();

    // A neutral third party pushes the claim through.
    const cranker = (await makeFundedWallet(provider, usdcMint, new BN(0), 0.05)).keypair;
    await program.methods
      .claimResult([])
      .accountsPartial({
        payer: cranker.publicKey,
        tournament: tournamentPda,
        matchAccount: semi0,
        nextMatch: nextPda,
        participantA: s0.participantA,
        participantB: s0.participantB,
        protocolConfig: protocolConfigPda,
        vault: vaultPda,
        organizerTokenAccount: null,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .signers([cranker])
      .rpc();

    const claimed = await program.account.matchNode.fetch(semi0);
    expect(claimed.status).to.deep.equal({ completed: {} });
    expect(claimed.winner.toBase58()).to.equal(s0.playerA.toBase58());
  });

  // ── 4. guards ─────────────────────────────────────────────────────────────
  it("rejects propose on OrganizerOnly tournaments", async () => {
    const { tournamentPda, players } = await setupStartedBracket({
      name: "pr-guard-mode",
      settlementMode: { organizerOnly: {} },
      disputeWindowSecs: 0,
    });
    const [semi0] = findMatchPda(tournamentPda, 0, 0, programId);
    const s0 = await finalizeAccounts(tournamentPda, semi0);
    await expectError(
      program.methods
        .proposeResult(s0.playerA)
        .accountsPartial({ proposer: s0.playerA, tournament: tournamentPda, matchAccount: semi0 })
        .signers([keypairFor(players, s0.playerA)])
        .rpc(),
      "SettlementModeMismatch",
    );
  });

  it("rejects confirm by the proposer, claim before window, claim after dispute, and early force-claim", async () => {
    const { tournamentPda, vaultPda, players } = await setupStartedBracket({
      name: "pr-guards",
      settlementMode: { playerReported: {} },
      disputeWindowSecs: 3600, // long window so claim is not yet eligible
    });
    const [semi0] = findMatchPda(tournamentPda, 0, 0, programId);
    const [nextPda] = findMatchPda(tournamentPda, 1, 0, programId);
    const s0 = await finalizeAccounts(tournamentPda, semi0);

    await program.methods
      .proposeResult(s0.playerA)
      .accountsPartial({ proposer: s0.playerA, tournament: tournamentPda, matchAccount: semi0 })
      .signers([keypairFor(players, s0.playerA)])
      .rpc();

    const finalize = {
      tournament: tournamentPda,
      matchAccount: semi0,
      nextMatch: nextPda,
      participantA: s0.participantA,
      participantB: s0.participantB,
      protocolConfig: protocolConfigPda,
      vault: vaultPda,
      organizerTokenAccount: null,
      tokenProgram: TOKEN_PROGRAM_ID,
    };

    // proposer cannot confirm their own proposal.
    await expectError(
      program.methods
        .confirmResult([])
        .accountsPartial({ counterparty: s0.playerA, ...finalize })
        .signers([keypairFor(players, s0.playerA)])
        .rpc(),
      "NotCounterparty",
    );

    // claim before the (long) window elapses.
    const cranker = (await makeFundedWallet(provider, usdcMint, new BN(0), 0.05)).keypair;
    await expectError(
      program.methods
        .claimResult([])
        .accountsPartial({ payer: cranker.publicKey, ...finalize })
        .signers([cranker])
        .rpc(),
      "ClaimWindowNotElapsed",
    );

    // dispute, then claim_result must refuse a disputed proposal.
    await program.methods
      .disputeResult(2)
      .accountsPartial({ disputer: s0.playerB, tournament: tournamentPda, matchAccount: semi0 })
      .signers([keypairFor(players, s0.playerB)])
      .rpc();
    await expectError(
      program.methods
        .claimResult([])
        .accountsPartial({ payer: cranker.publicKey, ...finalize })
        .signers([cranker])
        .rpc(),
      "ProposalDisputed",
    );

    // force-claim is gated behind the re-armed 24h deadline → too early now.
    await expectError(
      program.methods
        .forceClaimDisputed([])
        .accountsPartial({ payer: cranker.publicKey, ...finalize })
        .signers([cranker])
        .rpc(),
      "ClaimWindowNotElapsed",
    );
  });
});
