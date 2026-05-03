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
  MatchInitDescriptor,
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
// 5 strategic tests:
//   1. WTA happy path (8 players)              — proves end-to-end + 96.5/3.5 fee math
//   2. Standard happy path (8 players)         — proves multi-placement 60/25/15 split (DEMO flow)
//   3. Cancel + refund (4 players)             — proves money safety
//   4. 7-player with bye                       — proves non-power-of-2 bracket
//   5. 128-player chunked start                — proves scale claim (compute budget)
// ─────────────────────────────────────────────────────────────────────────────

describe("bracket-chain", function () {
  this.timeout(900_000);

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
  });

  // Helper: create a tournament + register N players. Returns wallets + PDAs.
  async function createAndJoin(opts: {
    name: string;
    payoutPreset: any; // anchor enum variant
    maxParticipants: number;
    playerCount: number;
  }) {
    // Organizer pays rent for Tournament + Vault + all MatchNode PDAs.
    // 128-player → 127 matches × ~0.00188 SOL rent ≈ 0.24 SOL → bump to 1.5 SOL safe.
    const organizerSol = opts.maxParticipants > 16 ? 1.5 : 0.1;
    const organizer = (await makeFundedWallet(provider, usdcMint, new BN(0), organizerSol)).keypair;
    const [tournamentPda] = findTournamentPda(organizer.publicKey, opts.name, programId);
    const [vaultPda] = findVaultPda(tournamentPda, programId);

    const deadline = new BN(Math.floor(Date.now() / 1000) + 3600);
    await program.methods
      .createTournament(
        opts.name,
        ENTRY_FEE,
        opts.maxParticipants,
        opts.payoutPreset,
        deadline,
        new BN(0),
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

  // Helper: report a non-final match. winner advances into next_match.
  async function reportNonFinal(
    organizer: Keypair,
    tournamentPda: PublicKey,
    round: number,
    matchIndex: number,
    nextRound: number,
    nextMatchIndex: number,
    winner: PublicKey,
    vaultPda: PublicKey,
  ) {
    const [matchPda] = findMatchPda(tournamentPda, round, matchIndex, programId);
    const [nextMatchPda] = findMatchPda(tournamentPda, nextRound, nextMatchIndex, programId);

    await program.methods
      .reportResult(winner, [])
      .accountsPartial({
        organizer: organizer.publicKey,
        tournament: tournamentPda,
        matchAccount: matchPda,
        nextMatch: nextMatchPda,
        protocolConfig: protocolConfigPda,
        vault: vaultPda,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .signers([organizer])
      .rpc();
  }

  // Helper: report the final match with placements + remaining_accounts ATAs.
  async function reportFinal(
    organizer: Keypair,
    tournamentPda: PublicKey,
    finalRound: number,
    placements: PublicKey[],
    placementAtas: PublicKey[],
    vaultPda: PublicKey,
  ) {
    const [finalMatchPda] = findMatchPda(tournamentPda, finalRound, 0, programId);
    const remaining: AccountMeta[] = [
      ...placementAtas.map((pubkey) => ({ pubkey, isSigner: false, isWritable: true })),
      { pubkey: treasuryAta, isSigner: false, isWritable: true },
    ];

    await program.methods
      .reportResult(placements[0], placements)
      .accountsPartial({
        organizer: organizer.publicKey,
        tournament: tournamentPda,
        matchAccount: finalMatchPda,
        nextMatch: null,
        protocolConfig: protocolConfigPda,
        vault: vaultPda,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .remainingAccounts(remaining)
      .signers([organizer])
      .rpc();
  }

  // ───────────────────────────────────────────────────────────────────────────
  // 1. WTA happy path — 8 players → champion gets 96.5%, treasury gets 3.5%
  // ───────────────────────────────────────────────────────────────────────────
  it("WTA: 8-player happy path distributes 96.5% to champion + 3.5% fee", async () => {
    const treasuryBefore = await tokenBalance(provider.connection, treasuryAta);

    const { organizer, tournamentPda, vaultPda, players } = await createAndJoin({
      name: "wta-8",
      payoutPreset: { winnerTakesAll: {} },
      maxParticipants: 8,
      playerCount: 8,
    });

    const playerKeys = players.map((p) => p.keypair.publicKey);
    const { descriptors, matchPdas } = buildBracketDescriptors(tournamentPda, playerKeys, programId);
    await sendStartChunks(program, organizer, tournamentPda, descriptors, matchPdas);

    // Round 0: winners are players[0,2,4,6] (left of each pair).
    await reportNonFinal(organizer, tournamentPda, 0, 0, 1, 0, playerKeys[0], vaultPda);
    await reportNonFinal(organizer, tournamentPda, 0, 1, 1, 0, playerKeys[2], vaultPda);
    await reportNonFinal(organizer, tournamentPda, 0, 2, 1, 1, playerKeys[4], vaultPda);
    await reportNonFinal(organizer, tournamentPda, 0, 3, 1, 1, playerKeys[6], vaultPda);
    // Round 1: winners are players[0,4].
    await reportNonFinal(organizer, tournamentPda, 1, 0, 2, 0, playerKeys[0], vaultPda);
    await reportNonFinal(organizer, tournamentPda, 1, 1, 2, 0, playerKeys[4], vaultPda);

    // Final
    const champion = playerKeys[0];
    await reportFinal(
      organizer,
      tournamentPda,
      2,
      [champion],
      [players[0].ata],
      vaultPda,
    );

    const grossPool = 8n * BigInt(ENTRY_FEE.toString());
    const expectedFee = (grossPool * 350n) / 10_000n;
    const expectedNet = grossPool - expectedFee;

    const treasuryAfter = await tokenBalance(provider.connection, treasuryAta);
    expect(await tokenBalance(provider.connection, players[0].ata)).to.equal(expectedNet);
    expect(treasuryAfter - treasuryBefore).to.equal(expectedFee);
    expect(await tokenBalance(provider.connection, vaultPda)).to.equal(0n);

    const t = await program.account.tournament.fetch(tournamentPda);
    expect(t.status).to.deep.equal({ completed: {} });
    expect(t.champion.toBase58()).to.equal(champion.toBase58());
  });

  // ───────────────────────────────────────────────────────────────────────────
  // 2. Standard happy path — 60/25/15 split + 3.5% fee (the DEMO flow)
  // ───────────────────────────────────────────────────────────────────────────
  it("Standard: 8-player happy path splits 60/25/15 + 3.5% fee", async () => {
    const treasuryBefore = await tokenBalance(provider.connection, treasuryAta);

    const { organizer, tournamentPda, vaultPda, players } = await createAndJoin({
      name: "std-8",
      payoutPreset: { standard: {} },
      maxParticipants: 8,
      playerCount: 8,
    });

    const playerKeys = players.map((p) => p.keypair.publicKey);
    const { descriptors, matchPdas } = buildBracketDescriptors(tournamentPda, playerKeys, programId);
    await sendStartChunks(program, organizer, tournamentPda, descriptors, matchPdas);

    // Decide outcomes:
    //   final: players[0] vs players[4] → players[0] wins (champ)
    //   semis: players[0] beats players[2]; players[4] beats players[6]
    //   so 2nd place (final loser) = players[4]
    //   3rd place = the semifinal loser we choose (organizer-trusted) = players[2]
    await reportNonFinal(organizer, tournamentPda, 0, 0, 1, 0, playerKeys[0], vaultPda);
    await reportNonFinal(organizer, tournamentPda, 0, 1, 1, 0, playerKeys[2], vaultPda);
    await reportNonFinal(organizer, tournamentPda, 0, 2, 1, 1, playerKeys[4], vaultPda);
    await reportNonFinal(organizer, tournamentPda, 0, 3, 1, 1, playerKeys[6], vaultPda);
    await reportNonFinal(organizer, tournamentPda, 1, 0, 2, 0, playerKeys[0], vaultPda);
    await reportNonFinal(organizer, tournamentPda, 1, 1, 2, 0, playerKeys[4], vaultPda);

    const placements = [playerKeys[0], playerKeys[4], playerKeys[2]];
    const placementAtas = [players[0].ata, players[4].ata, players[2].ata];
    await reportFinal(organizer, tournamentPda, 2, placements, placementAtas, vaultPda);

    const gross = 8n * BigInt(ENTRY_FEE.toString());
    const fee = (gross * 350n) / 10_000n;
    const net = gross - fee;
    const p1 = (net * 6000n) / 10_000n;
    const p2 = (net * 2500n) / 10_000n;
    const p3 = (net * 1500n) / 10_000n;

    const treasuryAfter = await tokenBalance(provider.connection, treasuryAta);
    expect(await tokenBalance(provider.connection, players[0].ata)).to.equal(p1);
    expect(await tokenBalance(provider.connection, players[4].ata)).to.equal(p2);
    expect(await tokenBalance(provider.connection, players[2].ata)).to.equal(p3);
    expect(treasuryAfter - treasuryBefore).to.equal(fee);
    // Vault drains (allow up to a few lamports of rounding remainder; bps math is exact for 8e6).
    expect(await tokenBalance(provider.connection, vaultPda)).to.equal(gross - p1 - p2 - p3 - fee);
  });

  // ───────────────────────────────────────────────────────────────────────────
  // 3. Cancel + refund — 4 joined → cancel → all refunded, vault drains
  // ───────────────────────────────────────────────────────────────────────────
  it("Cancel: 4 joined → cancel refunds all, vault drains to 0", async () => {
    const { organizer, tournamentPda, vaultPda, players } = await createAndJoin({
      name: "cancel-4",
      payoutPreset: { winnerTakesAll: {} },
      maxParticipants: 8,
      playerCount: 4,
    });

    const remaining: AccountMeta[] = [];
    for (const p of players) {
      remaining.push({ pubkey: p.participantPda, isSigner: false, isWritable: true });
      remaining.push({ pubkey: p.ata, isSigner: false, isWritable: true });
    }

    await program.methods
      .cancelTournament()
      .accountsPartial({
        caller: organizer.publicKey,
        tournament: tournamentPda,
        vault: vaultPda,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .remainingAccounts(remaining)
      .signers([organizer])
      .rpc();

    for (const p of players) {
      expect(await tokenBalance(provider.connection, p.ata)).to.equal(BigInt(ENTRY_FEE.toString()));
    }
    expect(await tokenBalance(provider.connection, vaultPda)).to.equal(0n);

    const t = await program.account.tournament.fetch(tournamentPda);
    expect(t.status).to.deep.equal({ cancelled: {} });
  });

  // ───────────────────────────────────────────────────────────────────────────
  // 4. 7-player with bye — bracket_size=8, top seed gets bye
  // ───────────────────────────────────────────────────────────────────────────
  it("Bye: 7-player tournament finishes with one round-0 bye", async () => {
    const treasuryBefore = await tokenBalance(provider.connection, treasuryAta);

    const { organizer, tournamentPda, vaultPda, players } = await createAndJoin({
      name: "bye-7",
      payoutPreset: { winnerTakesAll: {} },
      maxParticipants: 8,
      playerCount: 7,
    });

    // Pad with default to put bye at slot 0 (player_a = players[0]).
    // builder treats players[2m+1]==default as bye for players[2m].
    // We rearrange so default lands at index 1 (paired with players[0]).
    const playerKeys = [
      players[0].keypair.publicKey,
      PublicKey.default,
      players[1].keypair.publicKey,
      players[2].keypair.publicKey,
      players[3].keypair.publicKey,
      players[4].keypair.publicKey,
      players[5].keypair.publicKey,
      players[6].keypair.publicKey,
    ];

    // Give the builder a 7-player list; it'll pad with default at the end.
    // To force the bye into slot 0, prepend default into our arrangement instead.
    // Simpler: pass custom-sized array directly using bracketSize=8.
    const { descriptors, matchPdas } = buildBracketWithCustomSeats(
      tournamentPda,
      playerKeys,
      programId,
    );
    await sendStartChunks(program, organizer, tournamentPda, descriptors, matchPdas);

    // After init: round-0 match 0 = bye-completed (winner = players[0]).
    // Remaining round-0 matches: (1,2)→winner players[1]; (3,4)→players[3]; (5,6)→players[5].
    const pk = players.map((p) => p.keypair.publicKey);
    await reportNonFinal(organizer, tournamentPda, 0, 1, 1, 0, pk[1], vaultPda);
    await reportNonFinal(organizer, tournamentPda, 0, 2, 1, 1, pk[3], vaultPda);
    await reportNonFinal(organizer, tournamentPda, 0, 3, 1, 1, pk[5], vaultPda);
    // Round 1: (players[0] from bye, players[1]) → players[0]; (players[3], players[5]) → players[3]
    await reportNonFinal(organizer, tournamentPda, 1, 0, 2, 0, pk[0], vaultPda);
    await reportNonFinal(organizer, tournamentPda, 1, 1, 2, 0, pk[3], vaultPda);
    // Final
    await reportFinal(organizer, tournamentPda, 2, [pk[0]], [players[0].ata], vaultPda);

    const t = await program.account.tournament.fetch(tournamentPda);
    expect(t.status).to.deep.equal({ completed: {} });
    expect(t.bracketSize).to.equal(8);

    const gross = 7n * BigInt(ENTRY_FEE.toString());
    const fee = (gross * 350n) / 10_000n;
    const treasuryAfter = await tokenBalance(provider.connection, treasuryAta);
    expect(await tokenBalance(provider.connection, players[0].ata)).to.equal(gross - fee);
    expect(treasuryAfter - treasuryBefore).to.equal(fee);
  });

  // ───────────────────────────────────────────────────────────────────────────
  // 5. 128-player chunked start — proves compute-budget scale claim
  // ───────────────────────────────────────────────────────────────────────────
  it("128-player chunked start succeeds within compute budget", async function () {
    this.timeout(600_000);

    const { organizer, tournamentPda, players } = await createAndJoin({
      name: "scale-128",
      payoutPreset: { standard: {} },
      maxParticipants: 128,
      playerCount: 128,
    });

    const playerKeys = players.map((p) => p.keypair.publicKey);
    const { descriptors, matchPdas } = buildBracketDescriptors(tournamentPda, playerKeys, programId);

    // 127 matches across multiple chunks at chunk_size=7 → 19 chunks total.
    // See comment on sendStartChunks for the size-budget derivation
    // (chunk 8 was 1234 bytes — 2 over the 1232 legacy-tx limit).
    const sigs = await sendStartChunks(program, organizer, tournamentPda, descriptors, matchPdas, 7, 1_400_000);
    expect(sigs.length).to.be.greaterThan(1);

    const t = await program.account.tournament.fetch(tournamentPda);
    expect(t.status).to.deep.equal({ active: {} });
    expect(t.bracketSize).to.equal(128);
    expect(t.totalMatches).to.equal(127);
    expect(t.matchesInitialized).to.equal(127);
  });
});

// ─────────────────────────────────────────────────────────────────────────────
// Local helper: build descriptors when caller has already arranged seats
// (i.e. already placed defaults where byes should go).
// ─────────────────────────────────────────────────────────────────────────────
function buildBracketWithCustomSeats(
  tournament: PublicKey,
  seats: PublicKey[],
  programId: PublicKey,
): { descriptors: MatchInitDescriptor[]; matchPdas: PublicKey[] } {
  const bracketSize = seats.length;
  if ((bracketSize & (bracketSize - 1)) !== 0) {
    throw new Error("seats.length must be power of two");
  }
  const totalRounds = Math.log2(bracketSize);
  const descriptors: MatchInitDescriptor[] = [];
  const matchPdas: PublicKey[] = [];

  const round0 = bracketSize >> 1;
  const r0Bye: (PublicKey | null)[] = [];
  for (let m = 0; m < round0; m++) {
    const a = seats[2 * m];
    const b = seats[2 * m + 1];
    const isBye = a.equals(PublicKey.default) || b.equals(PublicKey.default);
    const playerA = isBye ? (a.equals(PublicKey.default) ? b : a) : a;
    const playerB = isBye ? PublicKey.default : b;
    const [pda, bump] = findMatchPda(tournament, 0, m, programId);
    descriptors.push({ round: 0, matchIndex: m, bump, playerA, playerB, bye: isBye });
    matchPdas.push(pda);
    r0Bye.push(isBye ? playerA : null);
  }

  for (let r = 1; r < totalRounds; r++) {
    const matches = bracketSize >> (r + 1);
    for (let m = 0; m < matches; m++) {
      let playerA = PublicKey.default;
      let playerB = PublicKey.default;
      if (r === 1) {
        playerA = r0Bye[2 * m] ?? PublicKey.default;
        playerB = r0Bye[2 * m + 1] ?? PublicKey.default;
      }
      const [pda, bump] = findMatchPda(tournament, r, m, programId);
      descriptors.push({ round: r, matchIndex: m, bump, playerA, playerB, bye: false });
      matchPdas.push(pda);
    }
  }

  return { descriptors, matchPdas };
}
