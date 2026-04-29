import * as anchor from "@coral-xyz/anchor";
import { BN, Program } from "@coral-xyz/anchor";
import {
  Keypair,
  PublicKey,
  SystemProgram,
  SYSVAR_SLOT_HASHES_PUBKEY,
  Connection,
  LAMPORTS_PER_SOL,
  TransactionInstruction,
  Transaction,
  AccountMeta,
  ComputeBudgetProgram,
} from "@solana/web3.js";
import {
  createMint,
  getOrCreateAssociatedTokenAccount,
  mintTo,
  TOKEN_PROGRAM_ID,
  getAccount,
  Account as TokenAccount,
} from "@solana/spl-token";
import { BracketChain } from "../target/types/bracket_chain";

export const USDC_DECIMALS = 6;
export const ENTRY_FEE = new BN(1_000_000); // 1 USDC

export type MatchInitDescriptor = {
  round: number;
  matchIndex: number;
  bump: number;
  playerA: PublicKey;
  playerB: PublicKey;
  bye: boolean;
};

// ─────────────────────────────────────────────────────────────────────────────
// PDAs
// ─────────────────────────────────────────────────────────────────────────────
export function findProtocolConfigPda(programId: PublicKey): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("protocol_config")],
    programId,
  );
}

export function findTournamentPda(
  organizer: PublicKey,
  name: string,
  programId: PublicKey,
): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("tournament"), organizer.toBuffer(), Buffer.from(name)],
    programId,
  );
}

export function findVaultPda(
  tournament: PublicKey,
  programId: PublicKey,
): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("vault"), tournament.toBuffer()],
    programId,
  );
}

export function findParticipantPda(
  tournament: PublicKey,
  wallet: PublicKey,
  programId: PublicKey,
): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("participant"), tournament.toBuffer(), wallet.toBuffer()],
    programId,
  );
}

export function findMatchPda(
  tournament: PublicKey,
  round: number,
  matchIndex: number,
  programId: PublicKey,
): [PublicKey, number] {
  const matchIndexLe = Buffer.alloc(2);
  matchIndexLe.writeUInt16LE(matchIndex, 0);
  return PublicKey.findProgramAddressSync(
    [
      Buffer.from("match"),
      tournament.toBuffer(),
      Buffer.from([round]),
      matchIndexLe,
    ],
    programId,
  );
}

// ─────────────────────────────────────────────────────────────────────────────
// SOL / mint / ATA helpers
// ─────────────────────────────────────────────────────────────────────────────
export async function fundFromProvider(
  provider: anchor.AnchorProvider,
  to: PublicKey,
  lamports: number,
): Promise<void> {
  const ix = SystemProgram.transfer({
    fromPubkey: provider.wallet.publicKey,
    toPubkey: to,
    lamports,
  });
  const tx = new Transaction().add(ix);
  await provider.sendAndConfirm(tx, []);
}

export async function createUsdcLikeMint(
  provider: anchor.AnchorProvider,
): Promise<PublicKey> {
  const payer = (provider.wallet as anchor.Wallet).payer;
  return await createMint(
    provider.connection,
    payer,
    provider.wallet.publicKey,
    null,
    USDC_DECIMALS,
  );
}

export async function makeFundedWallet(
  provider: anchor.AnchorProvider,
  mint: PublicKey,
  usdcAmount: BN,
  solAmount: number = 0.1,
): Promise<{ keypair: Keypair; ata: PublicKey }> {
  const kp = Keypair.generate();
  await fundFromProvider(provider, kp.publicKey, solAmount * LAMPORTS_PER_SOL);

  const payer = (provider.wallet as anchor.Wallet).payer;
  const ataInfo = await getOrCreateAssociatedTokenAccount(
    provider.connection,
    payer,
    mint,
    kp.publicKey,
  );

  if (!usdcAmount.isZero()) {
    await mintTo(
      provider.connection,
      payer,
      mint,
      ataInfo.address,
      provider.wallet.publicKey,
      BigInt(usdcAmount.toString()),
    );
  }

  return { keypair: kp, ata: ataInfo.address };
}

export async function makeAtaOnly(
  provider: anchor.AnchorProvider,
  mint: PublicKey,
  owner: PublicKey,
): Promise<PublicKey> {
  const payer = (provider.wallet as anchor.Wallet).payer;
  const ataInfo = await getOrCreateAssociatedTokenAccount(
    provider.connection,
    payer,
    mint,
    owner,
  );
  return ataInfo.address;
}

export async function tokenBalance(
  conn: Connection,
  ata: PublicKey,
): Promise<bigint> {
  const acc: TokenAccount = await getAccount(conn, ata);
  return acc.amount;
}

// ─────────────────────────────────────────────────────────────────────────────
// Bracket descriptor builder
// ─────────────────────────────────────────────────────────────────────────────
//
// Builds MatchInitDescriptors for a single-elim bracket given an ordered list
// of player pubkeys. `players.length` must satisfy 2 ≤ N ≤ bracketSize, where
// bracketSize is the next power of two ≥ N. If N < bracketSize, top seeds
// (lowest indices) get byes.
//
// Pairing: round-0 match i = players[2i] vs players[2i+1]. Byes are placed by
// padding players to bracketSize with `Pubkey.default` and treating any match
// where player_b == default as a bye for player_a.
//
// For round R > 0, descriptor.player_a/player_b are pre-populated *only* for
// slots fed by a bye in round R-1 (since reporting a real match auto-advances
// its winner via advance_winner). Other slots are left as default.
export function buildBracketDescriptors(
  tournament: PublicKey,
  players: PublicKey[],
  programId: PublicKey,
): { descriptors: MatchInitDescriptor[]; matchPdas: PublicKey[] } {
  const N = players.length;
  if (N < 2) throw new Error("need ≥2 players");

  const bracketSize = nextPowerOfTwo(N);
  const totalRounds = Math.log2(bracketSize); // e.g. 8 → 3
  const padded = [...players];
  while (padded.length < bracketSize) padded.push(PublicKey.default);

  // Track effective winner-known-at-init for each (round, matchIndex) so
  // round R+1 descriptors can pre-populate slots fed by byes.
  const winnerAtInit: PublicKey[][] = [];
  for (let r = 0; r < totalRounds; r++) {
    winnerAtInit.push(new Array(bracketSize >> (r + 1)).fill(PublicKey.default));
  }

  const descriptors: MatchInitDescriptor[] = [];
  const matchPdas: PublicKey[] = [];

  // Round 0
  const round0Matches = bracketSize >> 1;
  for (let m = 0; m < round0Matches; m++) {
    const a = padded[2 * m];
    const b = padded[2 * m + 1];
    const bye = a.equals(PublicKey.default) || b.equals(PublicKey.default);
    const playerA = bye ? (a.equals(PublicKey.default) ? b : a) : a;
    const playerB = bye ? PublicKey.default : b;
    const [pda, bump] = findMatchPda(tournament, 0, m, programId);
    descriptors.push({
      round: 0,
      matchIndex: m,
      bump,
      playerA,
      playerB,
      bye,
    });
    matchPdas.push(pda);
    if (bye) winnerAtInit[0][Math.floor(m / 2)] = playerA; // not used for r=0 itself
  }

  // Track parent winners propagated by byes for higher rounds
  // For round 0, `winnerAtInit[0][parent_idx]` records bye winners so we can
  // pre-fill round 1.
  const r0WinnersFromByes: (PublicKey | null)[] = [];
  for (let m = 0; m < round0Matches; m++) {
    const d = descriptors[m];
    r0WinnersFromByes.push(d.bye ? d.playerA : null);
  }

  // Higher rounds
  for (let r = 1; r < totalRounds; r++) {
    const matches = bracketSize >> (r + 1);
    for (let m = 0; m < matches; m++) {
      // Slots come from prior round matches 2m (left) and 2m+1 (right).
      // Pre-fill if the prior round's match was a bye-known winner.
      let playerA = PublicKey.default;
      let playerB = PublicKey.default;
      if (r === 1) {
        playerA = r0WinnersFromByes[2 * m] ?? PublicKey.default;
        playerB = r0WinnersFromByes[2 * m + 1] ?? PublicKey.default;
      }
      // For r ≥ 2, byes can't propagate past round 0 in a typical seeded
      // bracket of size ≤ 128 with a single bye row; if more byes exist this
      // helper would need extension. For our tests (max 1 bye row), default
      // is correct.
      const [pda, bump] = findMatchPda(tournament, r, m, programId);
      descriptors.push({
        round: r,
        matchIndex: m,
        bump,
        playerA,
        playerB,
        bye: false,
      });
      matchPdas.push(pda);
    }
  }

  return { descriptors, matchPdas };
}

export function nextPowerOfTwo(n: number): number {
  let p = 1;
  while (p < n) p <<= 1;
  return p;
}

// ─────────────────────────────────────────────────────────────────────────────
// start_tournament chunked sender
// ─────────────────────────────────────────────────────────────────────────────
// Chunk-size budget — measured empirically against legacy-tx 1232-byte limit.
//   • Anchor BorshInstructionCoder uses Buffer.alloc(1000) for ix args:
//     8 (disc) + 4 (Vec len) + N × 69 (descriptor) ≤ 1000  →  N ≤ 14
//   • Solana legacy tx-size hard cap = 1232 bytes. After signatures (65),
//     header+blockhash (~36), fixed accounts (organizer + tournament +
//     slot_hashes + system_program + compute_budget = ~165 bytes incl.
//     pubkeys & indices), compute-budget ix (~14), and start_tournament ix
//     framing (~30), we have ~920 bytes left.
//     Per match-PDA slot costs ~110 bytes (32 pubkey + 1 idx + 69 descriptor
//     + Vec/AccountMeta overhead). 920 / 110 ≈ 8.4  →  N=7 fits, N=8 spills
//     by 2 bytes (observed: 1234 > 1232).
//   • Default 7 leaves a comfortable margin. For 128-player turniру: 19 chunks.
//   • To raise the chunk size in V1: switch to versioned tx + Address Lookup
//     Table for the 4 fixed accounts (saves ~124 bytes → chunk 8-10 viable).
export async function sendStartChunks(
  program: Program<BracketChain>,
  organizer: Keypair,
  tournament: PublicKey,
  descriptors: MatchInitDescriptor[],
  matchPdas: PublicKey[],
  chunkSize: number = 7,
  computeUnits: number = 400_000,
): Promise<string[]> {
  const sigs: string[] = [];
  for (let i = 0; i < descriptors.length; i += chunkSize) {
    const dChunk = descriptors.slice(i, i + chunkSize);
    const pdaChunk = matchPdas.slice(i, i + chunkSize);
    const remainingAccounts: AccountMeta[] = pdaChunk.map((pubkey) => ({
      pubkey,
      isSigner: false,
      isWritable: true,
    }));
    const sig = await program.methods
      .startTournament(dChunk as any)
      .accountsPartial({
        organizer: organizer.publicKey,
        tournament,
        slotHashes: SYSVAR_SLOT_HASHES_PUBKEY,
        systemProgram: SystemProgram.programId,
      })
      .remainingAccounts(remainingAccounts)
      .preInstructions([
        ComputeBudgetProgram.setComputeUnitLimit({ units: computeUnits }),
      ])
      .signers([organizer])
      .rpc();
    sigs.push(sig);
  }
  return sigs;
}
