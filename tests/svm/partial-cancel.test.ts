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

// Stage E (E-2/E-3) — partial-cancel LiteSVM tier. Covers the instruction
// surface without a token CPI: `partial_cancel_tournament` status transitions
// (organizer-only, Active-gate) and `partial_refund_chunk`'s status gate +
// idempotent skip. The real refund transfer (vault → ATA) needs the SPL token
// program + a funded vault and runs at the Stage F **G8** validator acceptance
// (the refund loop is a near-verbatim copy of the validator-tested
// `cancel_tournament`).

const PROGRAM_ID = new PublicKey(
  "3YpkUKBh8288XN2dCKSwBnEdyc5UozSJ19A1ZCLpUZsZ"
);
const TOKEN_PROGRAM = new PublicKey(
  "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"
);

const ERR_UNAUTHORIZED = 6000;
const ERR_TOURNAMENT_IN_PROGRESS = 6011;

const stubProvider: any = { connection: {}, publicKey: PublicKey.default };
const program = new Program<BracketChain>(idl as any, stubProvider);
const coder = program.coder;

// ── PDAs ──────────────────────────────────────────────────────────────────
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
    entryFee: new BN(1_000_000),
    organizerDeposit: new BN(0),
    organizerDepositRefunded: false,
    maxParticipants: 4,
    bracketSize: 4,
    participantCount: 4,
    matchesInitialized: 3,
    matchesReported: 1,
    totalMatches: 3,
    registrationDeadline: new BN(0),
    createdAt: new BN(0),
    startedAt: new BN(1_000_000),
    completedAt: new BN(0),
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

function makeParticipant(opts: {
  tournament: PublicKey;
  wallet: PublicKey;
  bump: number;
  refundPaid: boolean;
  losses: number;
}) {
  return {
    tournament: opts.tournament,
    wallet: opts.wallet,
    seedIndex: 0,
    refundPaid: opts.refundPaid,
    bump: opts.bump,
    identityHash: Array(32).fill(0),
    identityAttestation: PublicKey.default,
    wins: 0,
    losses: opts.losses,
    pointsFor: 0,
    pointsAgainst: 0,
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
  name: "tournament" | "participant",
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
  mint: PublicKey,
  owner: PublicKey,
  amount: bigint
) {
  const buf = Buffer.alloc(165);
  mint.toBuffer().copy(buf, 0);
  owner.toBuffer().copy(buf, 32);
  buf.writeBigUInt64LE(amount, 64);
  buf.writeUInt8(1, 108);
  const lamports = svm.minimumBalanceForRentExemption(165n);
  svm.setAccount(
    pubkey.toBytes(),
    new LiteAccount(lamports, buf, TOKEN_PROGRAM.toBytes(), false, 0n)
  );
}
function buildIx(name: string, keys: AccountMeta[]): TransactionInstruction {
  return new TransactionInstruction({
    programId: PROGRAM_ID,
    keys,
    data: coder.instruction.encode(name, {}),
  });
}
function meta(
  pubkey: PublicKey,
  isSigner: boolean,
  isWritable: boolean
): AccountMeta {
  return { pubkey, isSigner, isWritable };
}
function sendTx(
  svm: LiteSvm,
  ix: TransactionInstruction,
  payer: Keypair,
  extra: Keypair[] = []
) {
  const tx = new Transaction().add(ix);
  tx.recentBlockhash = svm.latestBlockhash();
  tx.feePayer = payer.publicKey;
  tx.sign(payer, ...extra);
  return svm.sendLegacyTransaction(tx.serialize());
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
function expectOk(res: any) {
  if (res instanceof FailedTransactionMetadata)
    throw new Error(
      `expected success, got failure:\n${res.meta().logs().join("\n")}`
    );
}

async function setupTournament(
  svm: LiteSvm,
  organizer: PublicKey,
  status: any
) {
  const name = "PC Cup";
  const [tournament, bump] = tournamentPda(organizer, name);
  const [vault, vaultBump] = vaultPda(tournament);
  await writeAccount(
    svm,
    tournament,
    "tournament",
    makeTournament({ organizer, name, vault, bump, vaultBump, status })
  );
  writeTokenAccount(svm, vault, PublicKey.default, tournament, 4_000_000n);
  return { tournament, vault };
}

describe("partial-cancel (LiteSVM)", function () {
  describe("partial_cancel_tournament", function () {
    it("organizer flips Active → PartialCancelled", async () => {
      const svm = bootSvm();
      const organizer = new Keypair();
      svm.airdrop(organizer.publicKey.toBytes(), 10n ** 9n);
      const { tournament } = await setupTournament(svm, organizer.publicKey, {
        active: {},
      });

      const res = sendTx(
        svm,
        buildIx("partialCancelTournament", [
          meta(organizer.publicKey, true, false),
          meta(tournament, false, true),
        ]),
        organizer
      );
      expectOk(res);

      const acc = svm.getAccount(tournament.toBytes())!;
      const decoded: any = coder.accounts.decode(
        "tournament",
        Buffer.from(acc.data())
      );
      expect(Object.keys(decoded.status)[0]).to.equal("partialCancelled");
    });

    it("rejects a non-organizer signer", async () => {
      const svm = bootSvm();
      const organizer = new Keypair().publicKey;
      const stranger = new Keypair();
      svm.airdrop(stranger.publicKey.toBytes(), 10n ** 9n);
      const { tournament } = await setupTournament(svm, organizer, {
        active: {},
      });

      const res = sendTx(
        svm,
        buildIx("partialCancelTournament", [
          meta(stranger.publicKey, true, false),
          meta(tournament, false, true),
        ]),
        stranger
      );
      expectCustomErr(res, ERR_UNAUTHORIZED);
    });

    it("rejects when not Active (e.g. Registration)", async () => {
      const svm = bootSvm();
      const organizer = new Keypair();
      svm.airdrop(organizer.publicKey.toBytes(), 10n ** 9n);
      const { tournament } = await setupTournament(svm, organizer.publicKey, {
        registration: {},
      });

      const res = sendTx(
        svm,
        buildIx("partialCancelTournament", [
          meta(organizer.publicKey, true, false),
          meta(tournament, false, true),
        ]),
        organizer
      );
      expectCustomErr(res, ERR_TOURNAMENT_IN_PROGRESS);
    });
  });

  describe("partial_refund_chunk", function () {
    function refundKeys(
      caller: PublicKey,
      tournament: PublicKey,
      vault: PublicKey,
      pairs: PublicKey[][]
    ): AccountMeta[] {
      return [
        meta(caller, true, true),
        meta(tournament, false, true),
        meta(vault, false, true),
        meta(PROGRAM_ID, false, false), // organizer_token_account = None
        meta(TOKEN_PROGRAM, false, false),
        ...pairs.flatMap(([p, a]) => [
          meta(p, false, true),
          meta(a, false, true),
        ]),
      ];
    }

    it("rejects when status is not PartialCancelled", async () => {
      const svm = bootSvm();
      const caller = new Keypair();
      svm.airdrop(caller.publicKey.toBytes(), 10n ** 9n);
      const organizer = new Keypair().publicKey;
      const { tournament, vault } = await setupTournament(svm, organizer, {
        active: {},
      });

      const res = sendTx(
        svm,
        buildIx(
          "partialRefundChunk",
          refundKeys(caller.publicKey, tournament, vault, [])
        ),
        caller
      );
      expectCustomErr(res, ERR_TOURNAMENT_IN_PROGRESS);
    });

    it("skips an already-refunded participant (idempotent, no transfer)", async () => {
      const svm = bootSvm();
      const caller = new Keypair();
      svm.airdrop(caller.publicKey.toBytes(), 10n ** 9n);
      const organizer = new Keypair().publicKey;
      const { tournament, vault } = await setupTournament(svm, organizer, {
        partialCancelled: {},
      });

      const wallet = new Keypair().publicKey;
      const [pPda, pBump] = participantPda(tournament, wallet);
      await writeAccount(
        svm,
        pPda,
        "participant",
        makeParticipant({
          tournament,
          wallet,
          bump: pBump,
          refundPaid: true,
          losses: 1,
        })
      );
      const dummyAta = new Keypair().publicKey; // not touched — participant already refunded

      const res = sendTx(
        svm,
        buildIx(
          "partialRefundChunk",
          refundKeys(caller.publicKey, tournament, vault, [[pPda, dummyAta]])
        ),
        caller
      );
      expectOk(res);
    });
  });
});
