import * as anchor from "@coral-xyz/anchor";
import { BN, Program } from "@coral-xyz/anchor";
import { AccountMeta, PublicKey, SystemProgram } from "@solana/web3.js";
import { TOKEN_PROGRAM_ID } from "@solana/spl-token";
import { expect } from "chai";

import { BracketChain } from "../../target/types/bracket_chain";
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
  measureCu,
  rpcWithRetry,
  sendStartChunks,
  tokenBalance,
} from "./utils";

// ─────────────────────────────────────────────────────────────────────────────
// Phase 0 Section 3.4 — Tier-4 capacity + CU baseline.
// Runs the heaviest practical flow end-to-end:
//   • 128 players (max supported by program)
//   • Deep payout preset (7 placement payouts → most CPI fan-out)
//   • organizer_deposit > 0 (Variant B: deposit stays in the prize basis)
//   • Full bracket reported through to final-match payout
// Captures `meta.computeUnitsConsumed` on representative ix and asserts every
// sample stays under the 1_400_000 single-tx ceiling. CU samples become the
// Phase-0 baseline recorded in CU_BUDGET.md — Phase 1 redeploy ceremony
// (Step 88) re-runs this test and diffs against that file as a regression
// contract for the +83 bytes (MatchNode proposal envelope) + +74 bytes
// (Participant identity/stats) growth.
// ─────────────────────────────────────────────────────────────────────────────

const CU_CEILING = 1_400_000;
const ORGANIZER_DEPOSIT = new BN(10_000_000); // 10 USDC — non-trivial deposit

type CuRow = { ix: string; cu: number };

describe("capacity-128p-deep", function () {
  this.timeout(900_000);

  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);
  const program = anchor.workspace.bracketChain as Program<BracketChain>;
  const programId = program.programId;
  const conn = provider.connection;

  it("128p Deep — full bracket completes under 1.4M CU per ix", async () => {
    const usdcMint = await createUsdcLikeMint(provider);
    const init = await ensureProtocolInitialized(provider, program, usdcMint);
    const treasuryAta = init.treasuryAta;
    const protocolConfigPda = init.protocolConfigPda;

    const rows: CuRow[] = [];
    const record = async (label: string, sig: string) => {
      const cu = await measureCu(conn, sig);
      rows.push({ ix: label, cu });
      expect(cu, `${label} exceeded ${CU_CEILING} CU ceiling`).to.be.lessThan(
        CU_CEILING
      );
    };

    // Organizer needs SOL for: tournament rent + vault rent + 127 MatchNode
    // PDAs (~0.00188 SOL each) + chunked-start signatures. 2.0 SOL leaves
    // headroom for fee-payer churn across the full sweep.
    const organizer = await makeFundedWallet(
      provider,
      usdcMint,
      ORGANIZER_DEPOSIT,
      2.0
    );
    const tournamentName = "cap-128-deep";
    const [tournamentPda] = findTournamentPda(
      organizer.keypair.publicKey,
      tournamentName,
      programId
    );
    const [vaultPda] = findVaultPda(tournamentPda, programId);

    // ── 1. create_tournament (Deep + deposit) ──────────────────────────────
    const deadline = new BN(Math.floor(Date.now() / 1000) + 7200);
    const createSig = await rpcWithRetry(
      () =>
        program.methods
          .createTournament(
            tournamentName,
            ENTRY_FEE,
            128,
            { deep: {} } as any,
            deadline,
            ORGANIZER_DEPOSIT,
            { manual: {} }, // game: Manual
            { organizerOnly: {} }, // settlement_mode
            0 // dispute_window_secs
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
          .rpc(),
      "create_tournament"
    );
    await record("create_tournament (Deep, deposit=10 USDC)", createSig);

    // ── 2. join_tournament × 128 — sample CU at first, mid, last ───────────
    const players: {
      keypair: anchor.web3.Keypair;
      ata: PublicKey;
      participantPda: PublicKey;
    }[] = [];
    const joinSamples: { idx: number; sig: string }[] = [];
    for (let i = 0; i < 128; i++) {
      const w = await makeFundedWallet(provider, usdcMint, ENTRY_FEE);
      const [participantPda] = findParticipantPda(
        tournamentPda,
        w.keypair.publicKey,
        programId
      );
      const sig = await rpcWithRetry(
        () =>
          program.methods
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
            .rpc(),
        `join_tournament[${i}]`
      );
      players.push({ keypair: w.keypair, ata: w.ata, participantPda });
      if (i === 0 || i === 63 || i === 127) joinSamples.push({ idx: i, sig });
    }
    for (const s of joinSamples)
      await record(`join_tournament (player ${s.idx})`, s.sig);

    // ── 3. start_tournament — chunked. Sample first/mid/last chunk. ────────
    const playerKeys = players.map((p) => p.keypair.publicKey);
    const { descriptors, matchPdas } = buildBracketDescriptors(
      tournamentPda,
      playerKeys,
      programId
    );
    expect(descriptors.length).to.equal(127);

    const chunkSigs = await sendStartChunks(
      program,
      organizer.keypair,
      tournamentPda,
      descriptors,
      matchPdas,
      7,
      1_400_000
    );
    expect(chunkSigs.length).to.be.greaterThan(1);
    await record(
      `start_tournament (chunk 0/${chunkSigs.length})`,
      chunkSigs[0]
    );
    const midChunkIdx = Math.floor(chunkSigs.length / 2);
    await record(
      `start_tournament (chunk ${midChunkIdx}/${chunkSigs.length})`,
      chunkSigs[midChunkIdx]
    );
    await record(
      `start_tournament (chunk ${chunkSigs.length - 1}/${chunkSigs.length})`,
      chunkSigs[chunkSigs.length - 1]
    );

    // ── 4. report_result × 127 (126 non-final + 1 final) ───────────────────
    // Bracket: 128 players → 7 rounds. Round 0: 64 matches, R1: 32, R2: 16,
    // R3: 8, R4: 4, R5: 2, R6: 1 (final). Winner at every match: player_a
    // (the "left" seed) — deterministic so we can pre-compute placements.
    const winnerAtNode: PublicKey[][] = [];
    // Round 0: left seed of each pair wins.
    winnerAtNode.push([]);
    for (let m = 0; m < 64; m++) winnerAtNode[0].push(playerKeys[2 * m]);
    // Higher rounds: left child's winner advances.
    for (let r = 1; r <= 6; r++) {
      const matchCount = 1 << (6 - r);
      winnerAtNode.push([]);
      for (let m = 0; m < matchCount; m++) {
        winnerAtNode[r].push(winnerAtNode[r - 1][2 * m]);
      }
    }

    const reportSampleRounds = new Set([0, 3, 5]); // capture round-0, mid, last-non-final
    const reportSamples: { round: number; sig: string }[] = [];

    for (let r = 0; r < 6; r++) {
      const matchCount = 1 << (6 - r);
      for (let m = 0; m < matchCount; m++) {
        const [matchPda] = findMatchPda(tournamentPda, r, m, programId);
        const [nextMatchPda] = findMatchPda(
          tournamentPda,
          r + 1,
          Math.floor(m / 2),
          programId
        );
        const winner = winnerAtNode[r][m];
        const sig = await rpcWithRetry(
          () =>
            program.methods
              .reportResult(winner, [])
              .accountsPartial({
                organizer: organizer.keypair.publicKey,
                tournament: tournamentPda,
                matchAccount: matchPda,
                nextMatch: nextMatchPda,
                protocolConfig: protocolConfigPda,
                vault: vaultPda,
                tokenProgram: TOKEN_PROGRAM_ID,
              })
              .signers([organizer.keypair])
              .rpc(),
          `report_result[r${r},m${m}]`
        );
        if (reportSampleRounds.has(r) && m === 0) {
          reportSamples.push({ round: r, sig });
        }
      }
    }
    for (const s of reportSamples) {
      await record(
        `report_result non-final (round ${s.round}, match 0)`,
        s.sig
      );
    }

    // ── 5. report_result final (Deep, 7 placement payouts + treasury) ─────
    // Deep preset basis_points = [4000, 2500, 1500, 1000, 500, 300, 200]
    // → place 1..7. The semifinal losers and below come from organizer's
    // off-chain knowledge — here we synthesize them deterministically from
    // round-5/4 losers walking back from the final winner.
    // With "left seed always wins" pairing: champion = playerKeys[0]; the
    // final's other branch (R5 m1) winner = playerKeys[64] → runner-up.
    // Places 3..7 come from R0 right-seed losers — picked from indices that
    // are guaranteed distinct from the top 2 (1, 3, 5, 7, 9).
    const placementIdx = [0, 64, 1, 3, 5, 7, 9];
    const placements = placementIdx.map((i) => playerKeys[i]);
    const placementAtas = placementIdx.map((i) => players[i].ata);
    const champion = placements[0];

    const [finalMatchPda] = findMatchPda(tournamentPda, 6, 0, programId);
    const remaining: AccountMeta[] = [
      ...placementAtas.map((pubkey) => ({
        pubkey,
        isSigner: false,
        isWritable: true,
      })),
      { pubkey: treasuryAta, isSigner: false, isWritable: true },
    ];

    const treasuryBefore = await tokenBalance(conn, treasuryAta);
    const organizerBeforeFinal = await tokenBalance(conn, organizer.ata);
    const vaultBeforeFinal = await tokenBalance(conn, vaultPda);
    // Vault carries 128 entry fees + organizer deposit immediately before final.
    expect(vaultBeforeFinal).to.equal(
      128n * BigInt(ENTRY_FEE.toString()) + BigInt(ORGANIZER_DEPOSIT.toString())
    );

    const finalSig = await rpcWithRetry(
      () =>
        program.methods
          .reportResult(champion, placements)
          .accountsPartial({
            organizer: organizer.keypair.publicKey,
            tournament: tournamentPda,
            matchAccount: finalMatchPda,
            nextMatch: null,
            protocolConfig: protocolConfigPda,
            vault: vaultPda,
            tokenProgram: TOKEN_PROGRAM_ID,
          })
          .remainingAccounts(remaining)
          .signers([organizer.keypair])
          .rpc(),
      "report_result_FINAL"
    );
    await record("report_result FINAL (Deep, 128p, deposit in basis)", finalSig);

    // ── 6. Variant B invariants: deposit in the basis; nothing returns to
    //       the organizer; the dust-free split drains the vault to zero. ─────
    const grossBasis =
      128n * BigInt(ENTRY_FEE.toString()) + BigInt(ORGANIZER_DEPOSIT.toString());
    const feeExpected = (grossBasis * 350n) / 10_000n;
    const netExpected = grossBasis - feeExpected;
    const bps = [4000n, 2500n, 1500n, 1000n, 500n, 300n, 200n];
    const payouts = bps.map((b) => (netExpected * b) / 10_000n);
    // Champion (place 1) absorbs the floor-division remainder (Medium-1 split).
    const sumFloors = payouts.reduce((a, b) => a + b, 0n);
    payouts[0] += netExpected - sumFloors;

    const treasuryAfter = await tokenBalance(conn, treasuryAta);
    const organizerAfter = await tokenBalance(conn, organizer.ata);
    expect(treasuryAfter - treasuryBefore).to.equal(feeExpected);
    // Variant B: the deposit is prize money — the organizer gets nothing back.
    expect(organizerAfter - organizerBeforeFinal).to.equal(0n);

    expect(await tokenBalance(conn, players[0].ata)).to.equal(payouts[0]);
    // 7th place is the smallest, exercise the tail of the loop.
    expect(await tokenBalance(conn, players[9].ata)).to.equal(payouts[6]);

    // Dust-free split: the vault drains to exactly zero.
    expect(await tokenBalance(conn, vaultPda)).to.equal(0n);

    const t = await program.account.tournament.fetch(tournamentPda);
    expect(t.status).to.deep.equal({ completed: {} });
    expect(t.champion.toBase58()).to.equal(champion.toBase58());
    expect(t.organizerDepositRefunded).to.equal(false);

    // ── 7. Print CU table for CU_BUDGET.md ingestion ───────────────────────
    console.log("\n=== CU baseline (capacity-128p-deep) ===");
    for (const r of rows) {
      console.log(`  ${r.cu.toString().padStart(8)} CU  ${r.ix}`);
    }
    console.log(`  ceiling           ${CU_CEILING}`);
  });

  // Cross-preset baseline (WTA + Standard at 8p) lives in `bracket-chain.ts`
  // alongside the existing WTA/Standard tests — those run while the local
  // validator's blockhash cache is fresh. Attempting both 128p Deep and a
  // 2nd 8p flow in this file blew the blockhash store on solana-test-validator,
  // even with retry wrappers.
  it.skip("WTA + Standard final-match CU baseline (8p, no deposit)", async () => {
    const usdcMint = await createUsdcLikeMint(provider);
    const init = await ensureProtocolInitialized(provider, program, usdcMint);
    const treasuryAta = init.treasuryAta;
    const protocolConfigPda = init.protocolConfigPda;
    const rows: CuRow[] = [];

    async function runFinal(
      preset: "wta" | "standard",
      name: string
    ): Promise<void> {
      const placementCount = preset === "wta" ? 1 : 3;
      const organizer = await makeFundedWallet(
        provider,
        usdcMint,
        new BN(0),
        0.2
      );
      const [tournamentPda] = findTournamentPda(
        organizer.keypair.publicKey,
        name,
        programId
      );
      const [vaultPda] = findVaultPda(tournamentPda, programId);
      const deadline = new BN(Math.floor(Date.now() / 1000) + 3600);
      const presetArg: any =
        preset === "wta" ? { winnerTakesAll: {} } : { standard: {} };

      await rpcWithRetry(
        () =>
          program.methods
            .createTournament(
              name,
              ENTRY_FEE,
              8,
              presetArg,
              deadline,
              new BN(0),
              { manual: {} },
              { organizerOnly: {} },
              0
            )
            .accountsPartial({
              organizer: organizer.keypair.publicKey,
              protocolConfig: protocolConfigPda,
              tokenMint: usdcMint,
              tournament: tournamentPda,
              vault: vaultPda,
              organizerTokenAccount: null,
              tokenProgram: TOKEN_PROGRAM_ID,
              systemProgram: SystemProgram.programId,
              rent: anchor.web3.SYSVAR_RENT_PUBKEY,
            })
            .signers([organizer.keypair])
            .rpc(),
        `create_${preset}`
      );

      const playersPk: PublicKey[] = [];
      const playerAtas: PublicKey[] = [];
      for (let i = 0; i < 8; i++) {
        const w = await makeFundedWallet(provider, usdcMint, ENTRY_FEE);
        const [participantPda] = findParticipantPda(
          tournamentPda,
          w.keypair.publicKey,
          programId
        );
        await rpcWithRetry(
          () =>
            program.methods
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
              .rpc(),
          `join_${preset}_${i}`
        );
        playersPk.push(w.keypair.publicKey);
        playerAtas.push(w.ata);
      }

      const { descriptors, matchPdas } = buildBracketDescriptors(
        tournamentPda,
        playersPk,
        programId
      );
      await sendStartChunks(
        program,
        organizer.keypair,
        tournamentPda,
        descriptors,
        matchPdas
      );

      // Left seed always wins.
      for (let r = 0; r < 2; r++) {
        const matchCount = 1 << (2 - r);
        for (let m = 0; m < matchCount; m++) {
          const [matchPda] = findMatchPda(tournamentPda, r, m, programId);
          const [nextPda] = findMatchPda(
            tournamentPda,
            r + 1,
            Math.floor(m / 2),
            programId
          );
          // Winner of R0 m = playerKeys[2m]; R1 m = playerKeys[4m].
          const winner = playersPk[r === 0 ? 2 * m : 4 * m];
          await rpcWithRetry(
            () =>
              program.methods
                .reportResult(winner, [])
                .accountsPartial({
                  organizer: organizer.keypair.publicKey,
                  tournament: tournamentPda,
                  matchAccount: matchPda,
                  nextMatch: nextPda,
                  protocolConfig: protocolConfigPda,
                  vault: vaultPda,
                  tokenProgram: TOKEN_PROGRAM_ID,
                })
                .signers([organizer.keypair])
                .rpc(),
            `report_${preset}_r${r}_m${m}`
          );
        }
      }

      // Final: champion = playersPk[0]; runner-up = playersPk[4]; for Standard
      // 3rd-place is organizer-trusted — pick a semifinal loser.
      const placements =
        preset === "wta"
          ? [playersPk[0]]
          : [playersPk[0], playersPk[4], playersPk[2]];
      const placementAtaArr =
        preset === "wta"
          ? [playerAtas[0]]
          : [playerAtas[0], playerAtas[4], playerAtas[2]];
      const remaining: AccountMeta[] = [
        ...placementAtaArr.map((pubkey) => ({
          pubkey,
          isSigner: false,
          isWritable: true,
        })),
        { pubkey: treasuryAta, isSigner: false, isWritable: true },
      ];
      const [finalPda] = findMatchPda(tournamentPda, 2, 0, programId);
      const sig = await rpcWithRetry(
        () =>
          program.methods
            .reportResult(playersPk[0], placements)
            .accountsPartial({
              organizer: organizer.keypair.publicKey,
              tournament: tournamentPda,
              matchAccount: finalPda,
              nextMatch: null,
              protocolConfig: protocolConfigPda,
              vault: vaultPda,
              tokenProgram: TOKEN_PROGRAM_ID,
            })
            .remainingAccounts(remaining)
            .signers([organizer.keypair])
            .rpc(),
        `final_${preset}`
      );
      const cu = await measureCu(conn, sig);
      rows.push({
        ix: `report_result FINAL (${
          preset === "wta" ? "WTA" : "Standard"
        }, 8p, ${placementCount}+1 placement_atas)`,
        cu,
      });
      expect(cu).to.be.lessThan(CU_CEILING);
    }

    await runFinal("wta", "cu-wta-8");
    await runFinal("standard", "cu-std-8");

    console.log("\n=== CU baseline (cross-preset final-match) ===");
    for (const r of rows) {
      console.log(`  ${r.cu.toString().padStart(8)} CU  ${r.ix}`);
    }
  });
});
