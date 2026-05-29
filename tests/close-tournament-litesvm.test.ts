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

import idl from "../target/idl/bracket_chain.json";
import { BracketChain } from "../target/types/bracket_chain";

// Stage D (D-3) — `close_tournament` LiteSVM tier. Exercises the permissionless
// rent-reclaim instruction surface: the terminal-status gate, the child-PDA
// close (lamports → organizer), per-child validation, and idempotency. The
// root-close path (vault CPI + Tournament PDA) is left to the Stage F G7
// acceptance run on a real validator (needs a funded SPL vault round-trip);
// here we cover `close_root = false`, which is the bulk of the reclaimed rent.

const PROGRAM_ID = new PublicKey("3YpkUKBh8288XN2dCKSwBnEdyc5UozSJ19A1ZCLpUZsZ");
const TOKEN_PROGRAM = new PublicKey(
  "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA",
);

const ERR_UNAUTHORIZED = 6000;
const ERR_TOURNAMENT_IN_PROGRESS = 6011;
const ERR_INVALID_MATCH_INDEX = 6019;

const stubProvider: any = { connection: {}, publicKey: PublicKey.default };
const program = new Program<BracketChain>(idl as any, stubProvider);
const coder = program.coder;

// ── PDAs ──────────────────────────────────────────────────────────────────
function tournamentPda(organizer: PublicKey, name: string): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("tournament"), organizer.toBuffer(), Buffer.from(name)],
    PROGRAM_ID,
  );
}
function vaultPda(tournament: PublicKey): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("vault"), tournament.toBuffer()],
    PROGRAM_ID,
  );
}

// ── Fixtures ────────────────────────────────────────────────────────────────
function makeTournament(opts: {
  organizer: PublicKey;
  name: string;
  vault: PublicKey;
  bump: number;
  vaultBump: number;
  status: any;
}) {
  return {
    organizer: opts.organizer,
    name: opts.name,
    tokenMint: PublicKey.default,
    vault: opts.vault,
    entryFee: new BN(0),
    organizerDeposit: new BN(0),
    organizerDepositRefunded: false,
    maxParticipants: 4,
    bracketSize: 4,
    participantCount: 4,
    matchesInitialized: 3,
    matchesReported: 3,
    totalMatches: 3,
    registrationDeadline: new BN(0),
    createdAt: new BN(0),
    startedAt: new BN(1_000_000),
    completedAt: new BN(2_000_000),
    status: opts.status,
    payoutPreset: { winnerTakesAll: {} },
    seedHash: Array(32).fill(0),
    champion: PublicKey.default,
    bump: opts.bump,
    vaultBump: opts.vaultBump,
    game: { dota2: {} },
    settlementMode: { organizerOnly: {} },
    disputeWindowSecs: 0,
    vrfRandomnessAccount: PublicKey.default,
    vrfCommitSlot: new BN(0),
    seedRevealed: true,
    arbitrator: PublicKey.default,
  };
}

// ── LiteSVM helpers ─────────────────────────────────────────────────────────
function bootSvm(): LiteSvm {
  const svm = new LiteSvm();
  svm.setSysvars();
  svm.addProgramFromFile(
    PROGRAM_ID.toBytes(),
    path.join(__dirname, "..", "target", "deploy", "bracket_chain.so"),
  );
  return svm;
}

async function writeTournament(svm: LiteSvm, pubkey: PublicKey, obj: any) {
  const data = await coder.accounts.encode("tournament", obj);
  const lamports = svm.minimumBalanceForRentExemption(BigInt(data.length));
  svm.setAccount(
    pubkey.toBytes(),
    new LiteAccount(lamports, data, PROGRAM_ID.toBytes(), false, 0n),
  );
}

/** A 165-byte initialized SPL token account (state=Initialized) with `amount`. */
function writeTokenAccount(
  svm: LiteSvm,
  pubkey: PublicKey,
  mint: PublicKey,
  owner: PublicKey,
  amount: bigint,
) {
  const buf = Buffer.alloc(165);
  mint.toBuffer().copy(buf, 0);
  owner.toBuffer().copy(buf, 32);
  buf.writeBigUInt64LE(amount, 64);
  buf.writeUInt8(1, 108); // AccountState::Initialized
  const lamports = svm.minimumBalanceForRentExemption(165n);
  svm.setAccount(
    pubkey.toBytes(),
    new LiteAccount(lamports, buf, TOKEN_PROGRAM.toBytes(), false, 0n),
  );
}

/** A program-owned child PDA whose first field (bytes [8..40]) is `tournament`. */
function writeChild(svm: LiteSvm, pubkey: PublicKey, tournament: PublicKey) {
  const buf = Buffer.alloc(120);
  // 8-byte anchor discriminator can be anything for the close path (it only
  // reads bytes [8..40]); leave it zeroed.
  tournament.toBuffer().copy(buf, 8);
  const lamports = svm.minimumBalanceForRentExemption(120n);
  svm.setAccount(
    pubkey.toBytes(),
    new LiteAccount(lamports, buf, PROGRAM_ID.toBytes(), false, 0n),
  );
}

function buildIx(args: any, keys: AccountMeta[]): TransactionInstruction {
  const data = coder.instruction.encode("closeTournament", args);
  return new TransactionInstruction({ programId: PROGRAM_ID, keys, data });
}
function meta(pubkey: PublicKey, isSigner: boolean, isWritable: boolean): AccountMeta {
  return { pubkey, isSigner, isWritable };
}
function sendTx(svm: LiteSvm, ix: TransactionInstruction, payer: Keypair) {
  const tx = new Transaction().add(ix);
  tx.recentBlockhash = svm.latestBlockhash();
  tx.feePayer = payer.publicKey;
  tx.sign(payer);
  return svm.sendLegacyTransaction(tx.serialize());
}
function expectCustomErr(res: any, code: number) {
  if (!(res instanceof FailedTransactionMetadata)) {
    throw new Error(`expected failure (custom ${code}), got success`);
  }
  const e = res.err();
  if (!(e instanceof TransactionErrorInstructionError)) {
    throw new Error(`expected InstructionError, got: ${e.toString()}`);
  }
  const inner = e.err();
  if (!(inner instanceof InstructionErrorCustom)) {
    throw new Error(`expected Custom, got: ${inner.toString()}`);
  }
  expect(inner.code).to.equal(code);
}
function expectOk(res: any) {
  if (res instanceof FailedTransactionMetadata) {
    throw new Error(`expected success, got failure:\n${res.meta().logs().join("\n")}`);
  }
}

// ── Fixture ─────────────────────────────────────────────────────────────────
interface Fixture {
  svm: LiteSvm;
  caller: Keypair;
  organizer: PublicKey;
  tournament: PublicKey;
  vault: PublicKey;
}

async function setup(status: any): Promise<Fixture> {
  const svm = bootSvm();
  const caller = new Keypair();
  svm.airdrop(caller.publicKey.toBytes(), 10n ** 9n);

  const organizer = new Keypair().publicKey;
  const name = "Close Cup";
  const [tournament, bump] = tournamentPda(organizer, name);
  const [vault, vaultBump] = vaultPda(tournament);

  await writeTournament(
    svm,
    tournament,
    makeTournament({ organizer, name, vault, bump, vaultBump, status }),
  );
  writeTokenAccount(svm, vault, PublicKey.default, tournament, 0n);

  return { svm, caller, organizer, tournament, vault };
}

function baseKeys(fx: Fixture, remaining: PublicKey[]): AccountMeta[] {
  return [
    meta(fx.caller.publicKey, true, true),
    meta(fx.tournament, false, true),
    meta(fx.organizer, false, true),
    meta(fx.vault, false, true),
    meta(TOKEN_PROGRAM, false, false),
    ...remaining.map((pk) => meta(pk, false, true)),
  ];
}

describe("close_tournament (LiteSVM)", function () {
  it("rejects a non-terminal (Active) tournament", async () => {
    const fx = await setup({ active: {} });
    const child = new Keypair().publicKey;
    writeChild(fx.svm, child, fx.tournament);
    const res = sendTx(
      fx.svm,
      buildIx({ closeRoot: false }, baseKeys(fx, [child])),
      fx.caller,
    );
    expectCustomErr(res, ERR_TOURNAMENT_IN_PROGRESS);
  });

  it("closes child PDAs and routes their rent to the organizer", async () => {
    const fx = await setup({ cancelled: {} });
    const a = new Keypair().publicKey;
    const b = new Keypair().publicKey;
    writeChild(fx.svm, a, fx.tournament);
    writeChild(fx.svm, b, fx.tournament);

    const orgBefore = fx.svm.getBalance(fx.organizer.toBytes()) ?? 0n;
    const childRent = fx.svm.getBalance(a.toBytes())!;
    expect(childRent > 0n).to.equal(true);

    const res = sendTx(
      fx.svm,
      buildIx({ closeRoot: false }, baseKeys(fx, [a, b])),
      fx.caller,
    );
    expectOk(res);

    // Both children drained (closed).
    expect(fx.svm.getBalance(a.toBytes()) ?? 0n).to.equal(0n);
    expect(fx.svm.getBalance(b.toBytes()) ?? 0n).to.equal(0n);
    // Organizer received both children's rent.
    const orgAfter = fx.svm.getBalance(fx.organizer.toBytes()) ?? 0n;
    expect(orgAfter - orgBefore).to.equal(childRent * 2n);
  });

  it("is idempotent — re-closing an already-closed child is a no-op", async () => {
    const fx = await setup({ cancelled: {} });
    const a = new Keypair().publicKey;
    writeChild(fx.svm, a, fx.tournament);

    expectOk(sendTx(fx.svm, buildIx({ closeRoot: false }, baseKeys(fx, [a])), fx.caller));
    // New blockhash so the re-send isn't rejected as a duplicate signature.
    fx.svm.expireBlockhash();
    // Second pass: `a` already has 0 lamports → skipped, still succeeds.
    expectOk(sendTx(fx.svm, buildIx({ closeRoot: false }, baseKeys(fx, [a])), fx.caller));
  });

  it("rejects a child belonging to a different tournament", async () => {
    const fx = await setup({ cancelled: {} });
    const stranger = new Keypair().publicKey;
    writeChild(fx.svm, stranger, new Keypair().publicKey); // wrong tournament key
    const res = sendTx(
      fx.svm,
      buildIx({ closeRoot: false }, baseKeys(fx, [stranger])),
      fx.caller,
    );
    expectCustomErr(res, ERR_INVALID_MATCH_INDEX);
  });

  it("rejects when the organizer account is not the tournament organizer", async () => {
    const fx = await setup({ cancelled: {} });
    const wrongOrganizer = new Keypair().publicKey;
    const keys = [
      meta(fx.caller.publicKey, true, true),
      meta(fx.tournament, false, true),
      meta(wrongOrganizer, false, true),
      meta(fx.vault, false, true),
      meta(TOKEN_PROGRAM, false, false),
    ];
    const res = sendTx(fx.svm, buildIx({ closeRoot: false }, keys), fx.caller);
    expectCustomErr(res, ERR_UNAUTHORIZED);
  });
});
