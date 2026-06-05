import { BN, Program } from "@coral-xyz/anchor";
import {
  AccountMeta,
  Keypair,
  PublicKey,
  Transaction,
  TransactionInstruction,
} from "@solana/web3.js";
import {
  Account as LiteAccount,
  FailedTransactionMetadata,
  InstructionErrorCustom,
  LiteSvm,
  TransactionErrorInstructionError,
} from "litesvm/dist/internal";
import { expect } from "chai";
import * as path from "path";

import idl from "../../target/idl/bracket_chain.json";
import { BracketChain } from "../../target/types/bracket_chain";

// ─────────────────────────────────────────────────────────────────────────────
// organizer_deposit lifecycle — LiteSVM tier (down-migrated from the validator
// suite, Phase 0 §3.4). SPL Token is a LiteSVM built-in, so the real refund
// transfers (vault → ATA) run here without a validator. Accounts are pre-seeded
// in the target state and a single instruction is exercised.
//
// Coverage:
//   1. Pre-start cancel refunds the deposit to the organizer ATA + entry fees.
//   2. cancel_tournament is idempotent — the `organizer_deposit_refunded` guard
//      blocks a second refund.
//   3. Variant B (R13, ratified 2026-06-05) on report_result final-match: the
//      deposit STAYS in the vault — prize basis = full vault (entries +
//      deposit), the protocol fee applies to the deposit, and nothing returns
//      to the organizer (`organizer_deposit_refunded` stays false).
//   4. Sponsored prize pool — the Variant B motivating case: entry_fee = 0,
//      the deposit alone funds the prizes.
// ─────────────────────────────────────────────────────────────────────────────

const PROGRAM_ID = new PublicKey(
  (idl as any).address
);
const TOKEN_PROGRAM = new PublicKey(
  "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"
);

const ENTRY_FEE = 1_000_000n; // 1 USDC
const ORGANIZER_DEPOSIT = 5_000_000n; // 5 USDC
const FEE_BPS = 350n; // 3.5 %
const MINT = new PublicKey("Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB"); // any consistent mint

const stubProvider: any = { connection: {}, publicKey: PublicKey.default };
const program = new Program<BracketChain>(idl as any, stubProvider);
const coder = program.coder;

// ── PDAs ──────────────────────────────────────────────────────────────────────
function tournamentPda(
  organizer: PublicKey,
  name: string
): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("tournament"), organizer.toBuffer(), Buffer.from(name)],
    PROGRAM_ID
  );
}
function vaultPda(tournament: PublicKey): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("vault"), tournament.toBuffer()],
    PROGRAM_ID
  );
}
function participantPda(
  tournament: PublicKey,
  wallet: PublicKey
): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("participant"), tournament.toBuffer(), wallet.toBuffer()],
    PROGRAM_ID
  );
}
function protocolConfigPda(): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("protocol_config")],
    PROGRAM_ID
  );
}
function matchPda(
  tournament: PublicKey,
  round: number,
  idx: number
): [PublicKey, number] {
  const idxLe = Buffer.alloc(2);
  idxLe.writeUInt16LE(idx, 0);
  return PublicKey.findProgramAddressSync(
    [
      Buffer.from("match"),
      tournament.toBuffer(),
      Buffer.from([0]),
      Buffer.from([round]),
      idxLe,
    ],
    PROGRAM_ID
  );
}

// ── Fixtures ────────────────────────────────────────────────────────────────
function makeTournament(o: {
  organizer: PublicKey;
  name: string;
  vault: PublicKey;
  bump: number;
  vaultBump: number;
  status: any;
  organizerDeposit: bigint;
  organizerDepositRefunded: boolean;
  bracketSize: number;
  participantCount: number;
  matchesInitialized: number;
  matchesReported: number;
  totalMatches: number;
  /** Defaults to ENTRY_FEE; pass 0n for sponsored (deposit-only) pools. */
  entryFee?: bigint;
}) {
  return {
    organizer: o.organizer,
    name: o.name,
    tokenMint: MINT,
    vault: o.vault,
    entryFee: new BN((o.entryFee ?? ENTRY_FEE).toString()),
    organizerDeposit: new BN(o.organizerDeposit.toString()),
    organizerDepositRefunded: o.organizerDepositRefunded,
    maxParticipants: o.bracketSize,
    bracketSize: o.bracketSize,
    participantCount: o.participantCount,
    matchesInitialized: o.matchesInitialized,
    matchesReported: o.matchesReported,
    totalMatches: o.totalMatches,
    registrationDeadline: new BN(0),
    createdAt: new BN(0),
    startedAt: new BN(1_000_000),
    completedAt: new BN(0),
    status: o.status,
    payoutPreset: { winnerTakesAll: {} },
    seedHash: Array(32).fill(0),
    champion: PublicKey.default,
    bump: o.bump,
    vaultBump: o.vaultBump,
    game: { manual: {} },
    settlementMode: { organizerOnly: {} },
    disputeWindowSecs: 0,
    vrfRandomnessAccount: PublicKey.default,
    vrfCommitSlot: new BN(0),
    seedRevealed: true,
    arbitrator: PublicKey.default,
  };
}

function makeParticipant(o: {
  tournament: PublicKey;
  wallet: PublicKey;
  bump: number;
}) {
  return {
    tournament: o.tournament,
    wallet: o.wallet,
    seedIndex: 0,
    refundPaid: false,
    bump: o.bump,
    identityHash: Array(32).fill(0),
    identityAttestation: PublicKey.default,
    wins: 0,
    losses: 0,
    pointsFor: 0,
    pointsAgainst: 0,
  };
}

function makeMatch(o: {
  tournament: PublicKey;
  round: number;
  matchIndex: number;
  playerA: PublicKey;
  playerB: PublicKey;
  bump: number;
}) {
  return {
    tournament: o.tournament,
    bracket: 0,
    round: o.round,
    matchIndex: o.matchIndex,
    playerA: o.playerA,
    playerB: o.playerB,
    winner: PublicKey.default,
    status: { active: {} },
    bye: false,
    bump: o.bump,
    proposalSource: { none: {} },
    proposer: PublicKey.default,
    proposedWinner: PublicKey.default,
    proposedAt: new BN(0),
    claimDeadline: new BN(0),
    disputed: false,
    disputeReason: 0,
    commitment: null,
    switchboardFeed: PublicKey.default,
  };
}

function makeProtocolConfig(o: {
  authority: PublicKey;
  treasury: PublicKey;
  bump: number;
}) {
  return {
    authority: o.authority,
    treasury: o.treasury,
    defaultMint: MINT,
    feeBps: Number(FEE_BPS),
    bump: o.bump,
    sasCredential: PublicKey.default,
    sasSchemas: Array(5).fill(PublicKey.default),
    switchboardQueue: PublicKey.default,
    maxStaleSlots: 100,
    minOracleSamples: 5,
  };
}

// ── LiteSVM helpers ─────────────────────────────────────────────────────────
function bootSvm(): LiteSvm {
  const svm = new LiteSvm();
  svm.setSysvars();
  svm.addProgramFromFile(
    PROGRAM_ID.toBytes(),
    path.join(__dirname, "..", "..", "target", "deploy", "bracket_chain.so")
  );
  return svm;
}
async function writeAccount(
  svm: LiteSvm,
  pubkey: PublicKey,
  name: "tournament" | "participant" | "matchNode" | "protocolConfig",
  obj: any
) {
  const data = await coder.accounts.encode(name, obj);
  const lamports = svm.minimumBalanceForRentExemption(BigInt(data.length));
  svm.setAccount(
    pubkey.toBytes(),
    new LiteAccount(lamports, data, PROGRAM_ID.toBytes(), false, 0n)
  );
}
function writeTokenAccount(
  svm: LiteSvm,
  pubkey: PublicKey,
  owner: PublicKey,
  amount: bigint
) {
  const buf = Buffer.alloc(165);
  MINT.toBuffer().copy(buf, 0);
  owner.toBuffer().copy(buf, 32);
  buf.writeBigUInt64LE(amount, 64);
  buf.writeUInt8(1, 108); // state = Initialized
  const lamports = svm.minimumBalanceForRentExemption(165n);
  svm.setAccount(
    pubkey.toBytes(),
    new LiteAccount(lamports, buf, TOKEN_PROGRAM.toBytes(), false, 0n)
  );
}
function tokenAmount(svm: LiteSvm, pubkey: PublicKey): bigint {
  const acc = svm.getAccount(pubkey.toBytes());
  if (!acc) return 0n;
  return Buffer.from(acc.data()).readBigUInt64LE(64);
}
function meta(
  pubkey: PublicKey,
  isSigner: boolean,
  isWritable: boolean
): AccountMeta {
  return { pubkey, isSigner, isWritable };
}
function buildIx(
  name: string,
  keys: AccountMeta[],
  args: any = {}
): TransactionInstruction {
  return new TransactionInstruction({
    programId: PROGRAM_ID,
    keys,
    data: coder.instruction.encode(name, args),
  });
}
function sendTx(svm: LiteSvm, ix: TransactionInstruction, payer: Keypair) {
  const tx = new Transaction().add(ix);
  tx.recentBlockhash = svm.latestBlockhash();
  tx.feePayer = payer.publicKey;
  tx.sign(payer);
  return svm.sendLegacyTransaction(tx.serialize());
}
function expectOk(res: any) {
  if (res instanceof FailedTransactionMetadata) {
    throw new Error(
      `expected success, got failure:\n${res.meta().logs().join("\n")}`
    );
  }
}
function expectCustomErr(res: any, code: number) {
  if (!(res instanceof FailedTransactionMetadata))
    throw new Error(`expected failure (custom ${code}), got success`);
  const e = res.err();
  if (!(e instanceof TransactionErrorInstructionError))
    throw new Error(`expected InstructionError, got ${e.toString()}`);
  const inner = e.err();
  if (!(inner instanceof InstructionErrorCustom))
    throw new Error(`expected Custom, got ${inner.toString()}`);
  expect(inner.code).to.equal(code);
}
function decodeTournament(svm: LiteSvm, pubkey: PublicKey): any {
  const acc = svm.getAccount(pubkey.toBytes())!;
  return coder.accounts.decode("tournament", Buffer.from(acc.data()));
}

describe("organizer-deposit (LiteSVM)", function () {
  // ───────────────────────────────────────────────────────────────────────────
  // 1. Pre-start cancel refunds the deposit + entry fees
  // ───────────────────────────────────────────────────────────────────────────
  it("refunds organizer_deposit + entry fees on pre-start cancel", async () => {
    const svm = bootSvm();
    const organizer = new Keypair();
    svm.airdrop(organizer.publicKey.toBytes(), 10n ** 9n);
    const name = "od-cancel-1";
    const [tournament, bump] = tournamentPda(organizer.publicKey, name);
    const [vault, vaultBump] = vaultPda(tournament);

    await writeAccount(
      svm,
      tournament,
      "tournament",
      makeTournament({
        organizer: organizer.publicKey,
        name,
        vault,
        bump,
        vaultBump,
        status: { registration: {} },
        organizerDeposit: ORGANIZER_DEPOSIT,
        organizerDepositRefunded: false,
        bracketSize: 4,
        participantCount: 2,
        matchesInitialized: 0,
        matchesReported: 0,
        totalMatches: 3,
      })
    );
    writeTokenAccount(
      svm,
      vault,
      tournament,
      ORGANIZER_DEPOSIT + 2n * ENTRY_FEE
    );

    const p0 = new Keypair().publicKey;
    const p1 = new Keypair().publicKey;
    const [pPda0, pBump0] = participantPda(tournament, p0);
    const [pPda1, pBump1] = participantPda(tournament, p1);
    await writeAccount(
      svm,
      pPda0,
      "participant",
      makeParticipant({ tournament, wallet: p0, bump: pBump0 })
    );
    await writeAccount(
      svm,
      pPda1,
      "participant",
      makeParticipant({ tournament, wallet: p1, bump: pBump1 })
    );
    const ata0 = new Keypair().publicKey;
    const ata1 = new Keypair().publicKey;
    writeTokenAccount(svm, ata0, p0, 0n);
    writeTokenAccount(svm, ata1, p1, 0n);
    const organizerAta = new Keypair().publicKey;
    writeTokenAccount(svm, organizerAta, organizer.publicKey, 0n);

    const res = sendTx(
      svm,
      buildIx("cancelTournament", [
        meta(organizer.publicKey, true, true),
        meta(tournament, false, true),
        meta(vault, false, true),
        meta(organizerAta, false, true),
        meta(TOKEN_PROGRAM, false, false),
        meta(pPda0, false, true),
        meta(ata0, false, true),
        meta(pPda1, false, true),
        meta(ata1, false, true),
      ]),
      organizer
    );
    expectOk(res);

    expect(tokenAmount(svm, organizerAta)).to.equal(ORGANIZER_DEPOSIT);
    expect(tokenAmount(svm, ata0)).to.equal(ENTRY_FEE);
    expect(tokenAmount(svm, ata1)).to.equal(ENTRY_FEE);
    expect(tokenAmount(svm, vault)).to.equal(0n);
    const t = decodeTournament(svm, tournament);
    expect(Object.keys(t.status)[0]).to.equal("cancelled");
    expect(t.organizerDepositRefunded).to.equal(true);
  });

  // ───────────────────────────────────────────────────────────────────────────
  // 2. Idempotency — the refunded guard blocks a second deposit refund
  // ───────────────────────────────────────────────────────────────────────────
  it("does not double-refund the deposit (organizer_deposit_refunded guard)", async () => {
    const svm = bootSvm();
    const caller = new Keypair();
    svm.airdrop(caller.publicKey.toBytes(), 10n ** 9n);
    const organizer = new Keypair().publicKey;
    const name = "od-idemp-2";
    const [tournament, bump] = tournamentPda(organizer, name);
    const [vault, vaultBump] = vaultPda(tournament);

    // Post-first-cancel state: already Cancelled + refunded, vault drained.
    await writeAccount(
      svm,
      tournament,
      "tournament",
      makeTournament({
        organizer,
        name,
        vault,
        bump,
        vaultBump,
        status: { cancelled: {} },
        organizerDeposit: ORGANIZER_DEPOSIT,
        organizerDepositRefunded: true,
        bracketSize: 4,
        participantCount: 2,
        matchesInitialized: 0,
        matchesReported: 0,
        totalMatches: 3,
      })
    );
    writeTokenAccount(svm, vault, tournament, 0n);
    const organizerAta = new Keypair().publicKey;
    writeTokenAccount(svm, organizerAta, organizer, ORGANIZER_DEPOSIT); // already received

    const res = sendTx(
      svm,
      buildIx("cancelTournament", [
        meta(caller.publicKey, true, true),
        meta(tournament, false, true),
        meta(vault, false, true),
        meta(organizerAta, false, true),
        meta(TOKEN_PROGRAM, false, false),
      ]),
      caller
    );
    expectOk(res);

    // No second transfer — guard short-circuits.
    expect(tokenAmount(svm, organizerAta)).to.equal(ORGANIZER_DEPOSIT);
    expect(tokenAmount(svm, vault)).to.equal(0n);
    expect(decodeTournament(svm, tournament).organizerDepositRefunded).to.equal(
      true
    );
  });

  // ───────────────────────────────────────────────────────────────────────────
  // 3. Variant B — deposit included in the prize basis; never refunded on final
  // ───────────────────────────────────────────────────────────────────────────
  it("includes organizer_deposit in the prize basis on final-match (Variant B)", async () => {
    const svm = bootSvm();
    const organizer = new Keypair();
    svm.airdrop(organizer.publicKey.toBytes(), 10n ** 9n);
    const treasury = new Keypair().publicKey;
    const name = "od-final-3";
    const [tournament, bump] = tournamentPda(organizer.publicKey, name);
    const [vault, vaultBump] = vaultPda(tournament);
    const [pcfg, pcfgBump] = protocolConfigPda();
    // bracketSize 2 → the single round-0 match IS the final.
    const [finalMatch, mBump] = matchPda(tournament, 0, 0);

    const champ = new Keypair().publicKey; // player_a / winner
    const loser = new Keypair().publicKey;

    await writeAccount(
      svm,
      pcfg,
      "protocolConfig",
      makeProtocolConfig({
        authority: organizer.publicKey,
        treasury,
        bump: pcfgBump,
      })
    );
    await writeAccount(
      svm,
      tournament,
      "tournament",
      makeTournament({
        organizer: organizer.publicKey,
        name,
        vault,
        bump,
        vaultBump,
        status: { active: {} },
        organizerDeposit: ORGANIZER_DEPOSIT,
        organizerDepositRefunded: false,
        bracketSize: 2,
        participantCount: 2,
        matchesInitialized: 1,
        matchesReported: 0,
        totalMatches: 1,
      })
    );
    await writeAccount(
      svm,
      finalMatch,
      "matchNode",
      makeMatch({
        tournament,
        round: 0,
        matchIndex: 0,
        playerA: champ,
        playerB: loser,
        bump: mBump,
      })
    );

    writeTokenAccount(
      svm,
      vault,
      tournament,
      2n * ENTRY_FEE + ORGANIZER_DEPOSIT
    );
    const organizerAta = new Keypair().publicKey;
    const champAta = new Keypair().publicKey;
    const treasuryAta = new Keypair().publicKey;
    writeTokenAccount(svm, organizerAta, organizer.publicKey, 0n);
    writeTokenAccount(svm, champAta, champ, 0n);
    writeTokenAccount(svm, treasuryAta, treasury, 0n);

    const res = sendTx(
      svm,
      buildIx(
        "reportResult",
        [
          meta(organizer.publicKey, true, true),
          meta(tournament, false, true),
          meta(finalMatch, false, true),
          meta(PROGRAM_ID, false, false), // next_match = None
          meta(pcfg, false, false),
          meta(vault, false, true),
          meta(TOKEN_PROGRAM, false, false),
          meta(champAta, false, true), // remaining: placement[0]
          meta(treasuryAta, false, true), // remaining: treasury
        ],
        { winner: champ, placements: [champ] }
      ),
      organizer
    );
    expectOk(res);

    // Variant B: the prize basis is the FULL vault — entries + deposit.
    const basis = 2n * ENTRY_FEE + ORGANIZER_DEPOSIT;
    const expectedFee = (basis * FEE_BPS) / 10_000n;
    const expectedChampion = basis - expectedFee;

    expect(tokenAmount(svm, organizerAta)).to.equal(0n); // nothing returns
    expect(tokenAmount(svm, treasuryAta)).to.equal(expectedFee);
    expect(tokenAmount(svm, champAta)).to.equal(expectedChampion);
    expect(tokenAmount(svm, vault)).to.equal(0n);

    const t = decodeTournament(svm, tournament);
    expect(Object.keys(t.status)[0]).to.equal("completed");
    expect(t.champion.toBase58()).to.equal(champ.toBase58());
    expect(t.organizerDepositRefunded).to.equal(false);
  });

  // ───────────────────────────────────────────────────────────────────────────
  // 4. Sponsored prize pool — the Variant B motivating case. entry_fee = 0 and
  //    the organizer deposit alone funds the prizes; under Variant A this pool
  //    would have paid the champion nothing.
  // ───────────────────────────────────────────────────────────────────────────
  it("pays a deposit-only (sponsored) prize pool to the champion", async () => {
    const svm = bootSvm();
    const organizer = new Keypair();
    svm.airdrop(organizer.publicKey.toBytes(), 10n ** 9n);
    const treasury = new Keypair().publicKey;
    const name = "od-sponsored";
    const [tournament, bump] = tournamentPda(organizer.publicKey, name);
    const [vault, vaultBump] = vaultPda(tournament);
    const [pcfg, pcfgBump] = protocolConfigPda();
    const [finalMatch, mBump] = matchPda(tournament, 0, 0);
    const champ = new Keypair().publicKey;
    const loser = new Keypair().publicKey;

    await writeAccount(
      svm,
      pcfg,
      "protocolConfig",
      makeProtocolConfig({
        authority: organizer.publicKey,
        treasury,
        bump: pcfgBump,
      })
    );
    await writeAccount(
      svm,
      tournament,
      "tournament",
      makeTournament({
        organizer: organizer.publicKey,
        name,
        vault,
        bump,
        vaultBump,
        status: { active: {} },
        organizerDeposit: ORGANIZER_DEPOSIT,
        organizerDepositRefunded: false,
        bracketSize: 2,
        participantCount: 2,
        matchesInitialized: 1,
        matchesReported: 0,
        totalMatches: 1,
        entryFee: 0n, // free entry — the deposit IS the prize pool
      })
    );
    await writeAccount(
      svm,
      finalMatch,
      "matchNode",
      makeMatch({
        tournament,
        round: 0,
        matchIndex: 0,
        playerA: champ,
        playerB: loser,
        bump: mBump,
      })
    );
    // Vault holds only the sponsor deposit — no entries.
    writeTokenAccount(svm, vault, tournament, ORGANIZER_DEPOSIT);
    const champAta = new Keypair().publicKey;
    const treasuryAta = new Keypair().publicKey;
    writeTokenAccount(svm, champAta, champ, 0n);
    writeTokenAccount(svm, treasuryAta, treasury, 0n);

    const res = sendTx(
      svm,
      buildIx(
        "reportResult",
        [
          meta(organizer.publicKey, true, true),
          meta(tournament, false, true),
          meta(finalMatch, false, true),
          meta(PROGRAM_ID, false, false), // next_match = None
          meta(pcfg, false, false),
          meta(vault, false, true),
          meta(TOKEN_PROGRAM, false, false),
          meta(champAta, false, true), // remaining: placement[0]
          meta(treasuryAta, false, true), // remaining: treasury
        ],
        { winner: champ, placements: [champ] }
      ),
      organizer
    );
    expectOk(res);

    const expectedFee = (ORGANIZER_DEPOSIT * FEE_BPS) / 10_000n;
    expect(tokenAmount(svm, champAta)).to.equal(ORGANIZER_DEPOSIT - expectedFee);
    expect(tokenAmount(svm, treasuryAta)).to.equal(expectedFee);
    expect(tokenAmount(svm, vault)).to.equal(0n);
    expect(
      decodeTournament(svm, tournament).organizerDepositRefunded
    ).to.equal(false);
  });
});
