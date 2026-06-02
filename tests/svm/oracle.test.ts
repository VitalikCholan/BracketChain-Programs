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
  Clock,
  FailedTransactionMetadata,
  InstructionErrorCustom,
  LiteSvm,
  TransactionErrorInstructionError,
} from "litesvm/dist/internal";
import { expect } from "chai";
import * as path from "path";

import idl from "../../target/idl/bracket_chain.json";
import { BracketChain } from "../../target/types/bracket_chain";

// Stage C / V1.2 oracle settlement, LiteSVM tier (companion to the Rust-unit
// layer in `propose_result_oracle.rs::tests`). LiteSVM gives us the SBF runtime
// for free, so this layer exercises the *instruction surface* — Anchor account
// constraints, settlement-mode gates, and the feed-binding trust check — that
// the unit tests can't reach. The fixture writes Anchor accounts directly via
// `BorshCoder` (no `initialize_protocol` / `create_tournament` ceremony) so each
// test isolates exactly one branch.

const PROGRAM_ID = new PublicKey(
  "3YpkUKBh8288XN2dCKSwBnEdyc5UozSJ19A1ZCLpUZsZ"
);
const SWITCHBOARD_OD_DEVNET = new PublicKey(
  "Aio4gaXjXzJNVLtzwtNVmSqGKpANtXhybbkhtAC94ji2"
);

// PullFeedAccountData layout, measured against the live switchboard-on-demand
// 0.12.1 crate (Rust `mod layout_probe`, since removed). Total bytes = 3208
// (8-byte Anchor disc + 3200 Pod). 32 submission slots, each 64 bytes.
const PULLFEED_DISC = Buffer.from([196, 27, 108, 196, 10, 215, 219, 40]);
const PULLFEED_DATA_LEN = 3200;
const PULLFEED_OFF_QUEUE = 2080;
const PULLFEED_OFF_FEED_HASH = 2112;
const PULLFEED_SUB_SIZE = 64;
const PULLFEED_SUB_OFF_SLOT = 32;
const PULLFEED_SUB_OFF_LANDED = 40;
const PULLFEED_SUB_OFF_VALUE = 48;
// Switchboard scales submitted integers by 10^PRECISION (PRECISION = 18).
const SB_SCALE = 10n ** 18n;

// Error codes (target/idl/bracket_chain.json).
const ERR_PROPOSAL_EXISTS = 6036;
const ERR_WRONG_FEED = 6051;
const ERR_NOT_AUTHORIZED = 6053;

// ── Coder bootstrap ─────────────────────────────────────────────────────────
// `new Program(idl, provider)` runs `convertIdlToCamelCase` and builds a
// BorshCoder; we only need that coder. The stub provider is never reached —
// namespace construction stores the reference but doesn't touch `connection`.
const stubProvider: any = { connection: {}, publicKey: PublicKey.default };
const program = new Program<BracketChain>(idl as any, stubProvider);
const coder = program.coder;

// ── PDAs ────────────────────────────────────────────────────────────────────
function protocolConfigPda(): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("protocol_config")],
    PROGRAM_ID
  );
}
function tournamentPda(
  organizer: PublicKey,
  name: string
): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("tournament"), organizer.toBuffer(), Buffer.from(name)],
    PROGRAM_ID
  );
}
function matchPda(
  tournament: PublicKey,
  bracket: number,
  round: number,
  matchIndex: number
): [PublicKey, number] {
  const mi = Buffer.alloc(2);
  mi.writeUInt16LE(matchIndex, 0);
  return PublicKey.findProgramAddressSync(
    [
      Buffer.from("match"),
      tournament.toBuffer(),
      Buffer.from([bracket]),
      Buffer.from([round]),
      mi,
    ],
    PROGRAM_ID
  );
}

// ── Account fixtures (camelCase keys per convertIdlToCamelCase) ─────────────
function makeProtocolConfig(opts: {
  authority: PublicKey;
  treasury: PublicKey;
  defaultMint: PublicKey;
  bump: number;
  switchboardQueue: PublicKey;
}) {
  return {
    authority: opts.authority,
    treasury: opts.treasury,
    defaultMint: opts.defaultMint,
    feeBps: 250,
    bump: opts.bump,
    sasCredential: PublicKey.default,
    sasSchemas: Array(5).fill(PublicKey.default),
    switchboardQueue: opts.switchboardQueue,
    maxStaleSlots: 100,
    minOracleSamples: 5,
  };
}

function makeTournament(opts: {
  organizer: PublicKey;
  name: string;
  vault: PublicKey;
  bump: number;
  vaultBump: number;
  arbitrator: PublicKey;
  settlementMode: any;
  disputeWindowSecs: number;
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
    matchesReported: 0,
    totalMatches: 3,
    registrationDeadline: new BN(0),
    createdAt: new BN(0),
    startedAt: new BN(1_000_000),
    completedAt: new BN(0),
    status: { active: {} },
    payoutPreset: { winnerTakesAll: {} },
    seedHash: Array(32).fill(0),
    champion: PublicKey.default,
    bump: opts.bump,
    vaultBump: opts.vaultBump,
    game: { dota2: {} },
    settlementMode: opts.settlementMode,
    disputeWindowSecs: opts.disputeWindowSecs,
    vrfRandomnessAccount: PublicKey.default,
    vrfCommitSlot: new BN(0),
    seedRevealed: true,
    arbitrator: opts.arbitrator,
  };
}

type CommitmentObj = {
  lobbyId: number[];
  playerAGameId: number[];
  playerBGameId: number[];
  expectedFeedHash: number[];
};

function makeMatchNode(opts: {
  tournament: PublicKey;
  bracket: number;
  round: number;
  matchIndex: number;
  bump: number;
  playerA: PublicKey;
  playerB: PublicKey;
  commitment?: CommitmentObj;
  switchboardFeed?: PublicKey;
  proposalSource?: any;
  proposer?: PublicKey;
  proposedWinner?: PublicKey;
}) {
  return {
    tournament: opts.tournament,
    bracket: opts.bracket,
    round: opts.round,
    matchIndex: opts.matchIndex,
    playerA: opts.playerA,
    playerB: opts.playerB,
    winner: PublicKey.default,
    status: { active: {} },
    bye: false,
    bump: opts.bump,
    proposalSource: opts.proposalSource ?? { none: {} },
    proposer: opts.proposer ?? PublicKey.default,
    proposedWinner: opts.proposedWinner ?? PublicKey.default,
    proposedAt: new BN(0),
    claimDeadline: new BN(0),
    disputed: false,
    disputeReason: 0,
    commitment: opts.commitment
      ? {
          ...opts.commitment,
          committedAt: new BN(0),
          committedSlot: new BN(0),
        }
      : null,
    switchboardFeed: opts.switchboardFeed ?? PublicKey.default,
  };
}

// ── LiteSVM helpers ─────────────────────────────────────────────────────────
function bootSvm(): LiteSvm {
  const svm = new LiteSvm();
  svm.setSysvars();
  const soPath = path.join(
    __dirname,
    "..",
    "..",
    "target",
    "deploy",
    "bracket_chain.so"
  );
  svm.addProgramFromFile(PROGRAM_ID.toBytes(), soPath);
  return svm;
}

async function writeAnchorAccount(
  svm: LiteSvm,
  pubkey: PublicKey,
  name: "tournament" | "matchNode" | "protocolConfig",
  obj: any
) {
  const data = await coder.accounts.encode(name, obj);
  const lamports = svm.minimumBalanceForRentExemption(BigInt(data.length));
  svm.setAccount(
    pubkey.toBytes(),
    new LiteAccount(lamports, data, PROGRAM_ID.toBytes(), false, 0n)
  );
}

function writePullFeed(
  svm: LiteSvm,
  pubkey: PublicKey,
  queue: PublicKey,
  feedHash: Uint8Array,
  valueI128: bigint,
  landedAtSlot: bigint,
  nSamples: number
) {
  const buf = Buffer.alloc(8 + PULLFEED_DATA_LEN);
  PULLFEED_DISC.copy(buf, 0);
  const dataOff = 8;
  for (let i = 0; i < nSamples; i++) {
    const subOff = dataOff + i * PULLFEED_SUB_SIZE;
    buf.writeBigUInt64LE(landedAtSlot, subOff + PULLFEED_SUB_OFF_SLOT);
    buf.writeBigInt64LE(landedAtSlot, subOff + PULLFEED_SUB_OFF_LANDED);
    writeI128LE(buf, subOff + PULLFEED_SUB_OFF_VALUE, valueI128);
  }
  queue.toBuffer().copy(buf, dataOff + PULLFEED_OFF_QUEUE);
  Buffer.from(feedHash).copy(buf, dataOff + PULLFEED_OFF_FEED_HASH);

  const lamports = svm.minimumBalanceForRentExemption(BigInt(buf.length));
  svm.setAccount(
    pubkey.toBytes(),
    new LiteAccount(lamports, buf, SWITCHBOARD_OD_DEVNET.toBytes(), false, 0n)
  );
}

function writeI128LE(buf: Buffer, offset: number, value: bigint) {
  // Positive values only (winner index ≥ 0 × 10^18); two's-complement sign
  // extension not needed for our test inputs.
  const mask = (1n << 64n) - 1n;
  buf.writeBigUInt64LE(value & mask, offset);
  buf.writeBigUInt64LE((value >> 64n) & mask, offset + 8);
}

function setSlot(svm: LiteSvm, slot: bigint, unixTs: bigint) {
  const c = svm.getClock();
  svm.setClock(
    new Clock(
      slot,
      c.epochStartTimestamp,
      c.epoch,
      c.leaderScheduleEpoch,
      unixTs
    )
  );
}

function randomBytes(n: number): Uint8Array {
  const arr = new Uint8Array(n);
  for (let i = 0; i < n; i++) arr[i] = Math.floor(Math.random() * 256);
  return arr;
}

// ── Instruction + tx ────────────────────────────────────────────────────────
function buildIx(
  name: string,
  args: any,
  keys: AccountMeta[]
): TransactionInstruction {
  const data = coder.instruction.encode(name, args);
  return new TransactionInstruction({ programId: PROGRAM_ID, keys, data });
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
  extraSigners: Keypair[] = []
) {
  const tx = new Transaction().add(ix);
  tx.recentBlockhash = svm.latestBlockhash();
  tx.feePayer = payer.publicKey;
  tx.sign(payer, ...extraSigners);
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
  expect(inner.code).to.equal(
    code,
    `wrong custom code (logs:\n${res.meta().logs().join("\n")})`
  );
}

function expectOk(res: any) {
  if (res instanceof FailedTransactionMetadata) {
    throw new Error(
      `tx failed: ${res.err().toString()}\nlogs:\n${res
        .meta()
        .logs()
        .join("\n")}`
    );
  }
}

// ── Fixture ─────────────────────────────────────────────────────────────────
type Fixture = {
  svm: LiteSvm;
  organizer: Keypair;
  arbitrator: Keypair;
  playerA: Keypair;
  playerB: Keypair;
  relayer: Keypair;
  tournament: PublicKey;
  matchAcc: PublicKey;
  protocolConfig: PublicKey;
  switchboardQueue: PublicKey;
  feedKeypair: Keypair;
  expectedFeedHash: Uint8Array;
};

async function setupOracleFixture(
  over: Partial<{
    proposalSource: any;
    expectedFeedHash: Uint8Array;
  }> = {}
): Promise<Fixture> {
  const svm = bootSvm();

  const organizer = Keypair.generate();
  const playerA = Keypair.generate();
  const playerB = Keypair.generate();
  const relayer = Keypair.generate();
  const switchboardQueue = Keypair.generate().publicKey;
  const feedKeypair = Keypair.generate();
  const expectedFeedHash = over.expectedFeedHash ?? randomBytes(32);

  for (const kp of [organizer, playerA, playerB, relayer]) {
    svm.airdrop(kp.publicKey.toBytes(), 10n ** 9n);
  }

  const [protocolConfig, pcBump] = protocolConfigPda();
  await writeAnchorAccount(
    svm,
    protocolConfig,
    "protocolConfig",
    makeProtocolConfig({
      authority: organizer.publicKey,
      treasury: PublicKey.default,
      defaultMint: PublicKey.default,
      bump: pcBump,
      switchboardQueue,
    })
  );

  // Tournament name short enough that Tournament fits in BorshCoder's 1000-byte
  // scratch buffer with room to spare.
  const tName = "ora-" + organizer.publicKey.toBase58().slice(0, 8);
  const [tournament, tBump] = tournamentPda(organizer.publicKey, tName);
  await writeAnchorAccount(
    svm,
    tournament,
    "tournament",
    makeTournament({
      organizer: organizer.publicKey,
      name: tName,
      vault: Keypair.generate().publicKey,
      bump: tBump,
      vaultBump: 255,
      arbitrator: organizer.publicKey,
      settlementMode: { oracle: {} },
      disputeWindowSecs: 60,
    })
  );

  const [matchAcc, mBump] = matchPda(tournament, 0, 0, 0);
  const commitment: CommitmentObj = {
    lobbyId: Array(16).fill(7),
    playerAGameId: Array(32).fill(1),
    playerBGameId: Array(32).fill(2),
    expectedFeedHash: Array.from(expectedFeedHash),
  };
  await writeAnchorAccount(
    svm,
    matchAcc,
    "matchNode",
    makeMatchNode({
      tournament,
      bracket: 0,
      round: 0,
      matchIndex: 0,
      bump: mBump,
      playerA: playerA.publicKey,
      playerB: playerB.publicKey,
      commitment,
      switchboardFeed: feedKeypair.publicKey,
      proposalSource: over.proposalSource,
      proposer: over.proposalSource ? relayer.publicKey : undefined,
      proposedWinner: over.proposalSource ? playerA.publicKey : undefined,
    })
  );

  return {
    svm,
    organizer,
    arbitrator: organizer,
    playerA,
    playerB,
    relayer,
    tournament,
    matchAcc,
    protocolConfig,
    switchboardQueue,
    feedKeypair,
    expectedFeedHash,
  };
}

// ── Tests ───────────────────────────────────────────────────────────────────
describe("oracle settlement (LiteSVM)", function () {
  this.timeout(60_000);

  it("propose_result_oracle: feed reporting 1 → proposed_winner = player_b, source = Oracle", async () => {
    const fx = await setupOracleFixture();

    setSlot(fx.svm, 100n, 1_700_000_000n);
    writePullFeed(
      fx.svm,
      fx.feedKeypair.publicKey,
      fx.switchboardQueue,
      fx.expectedFeedHash,
      1n * SB_SCALE,
      100n,
      5
    );

    const ix = buildIx("proposeResultOracle", {}, [
      meta(fx.relayer.publicKey, true, false),
      meta(fx.tournament, false, false),
      meta(fx.matchAcc, false, true),
      meta(fx.protocolConfig, false, false),
      meta(fx.feedKeypair.publicKey, false, false),
    ]);
    const res = sendTx(fx.svm, ix, fx.relayer);
    expectOk(res);

    const acc = fx.svm.getAccount(fx.matchAcc.toBytes())!;
    const decoded: any = coder.accounts.decode(
      "matchNode",
      Buffer.from(acc.data())
    );
    expect(decoded.proposalSource).to.deep.equal({ oracle: {} });
    expect(decoded.proposedWinner.toBase58()).to.equal(
      fx.playerB.publicKey.toBase58()
    );
    expect(decoded.proposer.toBase58()).to.equal(
      fx.relayer.publicKey.toBase58()
    );
  });

  it("propose_result_oracle: wrong feed key → WrongFeedAccount", async () => {
    const fx = await setupOracleFixture();
    const wrongFeed = Keypair.generate();
    setSlot(fx.svm, 100n, 1_700_000_000n);
    // No need to back `wrongFeed` with data — the keys-eq check fires before
    // any parse. (Solana exposes missing accounts as zero-lamport / 0-byte.)

    const ix = buildIx("proposeResultOracle", {}, [
      meta(fx.relayer.publicKey, true, false),
      meta(fx.tournament, false, false),
      meta(fx.matchAcc, false, true),
      meta(fx.protocolConfig, false, false),
      meta(wrongFeed.publicKey, false, false),
    ]);
    const res = sendTx(fx.svm, ix, fx.relayer);
    expectCustomErr(res, ERR_WRONG_FEED);
  });

  it("propose_result_oracle: already-proposed → ProposalAlreadyExists", async () => {
    const fx = await setupOracleFixture({ proposalSource: { player: {} } });

    const ix = buildIx("proposeResultOracle", {}, [
      meta(fx.relayer.publicKey, true, false),
      meta(fx.tournament, false, false),
      meta(fx.matchAcc, false, true),
      meta(fx.protocolConfig, false, false),
      meta(fx.feedKeypair.publicKey, false, false),
    ]);
    const res = sendTx(fx.svm, ix, fx.relayer);
    expectCustomErr(res, ERR_PROPOSAL_EXISTS);
  });

  it("bind_match_feed: feed_hash mismatch → WrongFeedAccount", async () => {
    const fx = await setupOracleFixture();
    const wrongHash = randomBytes(32);
    writePullFeed(
      fx.svm,
      fx.feedKeypair.publicKey,
      fx.switchboardQueue,
      wrongHash,
      1n * SB_SCALE,
      100n,
      5
    );

    const ix = buildIx("bindMatchFeed", {}, [
      meta(fx.organizer.publicKey, true, false),
      meta(fx.tournament, false, false),
      meta(fx.matchAcc, false, true),
      meta(fx.protocolConfig, false, false),
      meta(fx.feedKeypair.publicKey, false, false),
    ]);
    const res = sendTx(fx.svm, ix, fx.organizer);
    expectCustomErr(res, ERR_WRONG_FEED);
  });

  describe("dispute_result broadening (Oracle proposal)", () => {
    it("player_a can dispute an Oracle proposal", async () => {
      const fx = await setupOracleFixture({ proposalSource: { oracle: {} } });
      const ix = buildIx("disputeResult", { disputeReason: 0 }, [
        meta(fx.playerA.publicKey, true, false),
        meta(fx.tournament, false, false),
        meta(fx.matchAcc, false, true),
      ]);
      const res = sendTx(fx.svm, ix, fx.playerA);
      expectOk(res);
      const acc = fx.svm.getAccount(fx.matchAcc.toBytes())!;
      const decoded: any = coder.accounts.decode(
        "matchNode",
        Buffer.from(acc.data())
      );
      expect(decoded.disputed).to.equal(true);
    });

    it("arbitrator (= organizer) can dispute an Oracle proposal", async () => {
      const fx = await setupOracleFixture({ proposalSource: { oracle: {} } });
      const ix = buildIx("disputeResult", { disputeReason: 0 }, [
        meta(fx.arbitrator.publicKey, true, false),
        meta(fx.tournament, false, false),
        meta(fx.matchAcc, false, true),
      ]);
      const res = sendTx(fx.svm, ix, fx.arbitrator);
      expectOk(res);
    });

    it("unrelated wallet cannot dispute → NotAuthorized", async () => {
      const fx = await setupOracleFixture({ proposalSource: { oracle: {} } });
      const stranger = Keypair.generate();
      fx.svm.airdrop(stranger.publicKey.toBytes(), 10n ** 9n);
      const ix = buildIx("disputeResult", { disputeReason: 0 }, [
        meta(stranger.publicKey, true, false),
        meta(fx.tournament, false, false),
        meta(fx.matchAcc, false, true),
      ]);
      const res = sendTx(fx.svm, ix, stranger);
      expectCustomErr(res, ERR_NOT_AUTHORIZED);
    });
  });
});
