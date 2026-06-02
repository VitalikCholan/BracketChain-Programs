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
// H-1 — multi-placement final hardening (settle_final + finalize gate), LiteSVM
// tier (down-migrated from the validator suite). Single-elim has no 3rd-place
// match, so placements[2..] are organizer-trusted: the finalize gate rejects a
// multi-placement (non-WTA) final on every permissionless / counterparty path,
// and only the arbitrator-signed `settle_final` may adjudicate placements 3..N.
// WTA finals stay permissionlessly claimable.
//
// Final-only state is pre-seeded (bracketSize 2 → the single match is the
// final); a real `propose_result` pins the trustless winner.
// ─────────────────────────────────────────────────────────────────────────────

const PROGRAM_ID = new PublicKey(
  "3YpkUKBh8288XN2dCKSwBnEdyc5UozSJ19A1ZCLpUZsZ"
);
const TOKEN_PROGRAM = new PublicKey(
  "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"
);

const ENTRY_FEE = 1_000_000n;
const FEE_BPS = 350n;
const GROSS = 4n * ENTRY_FEE; // vault funded to match the validator suite's math
const MINT = new PublicKey("Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB");

const ERR_UNAUTHORIZED = 6000;
const ERR_UNTRUSTED_MULTI_PLACEMENT_FINAL = 6056;

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
  arbitrator: PublicKey;
  name: string;
  vault: PublicKey;
  bump: number;
  vaultBump: number;
  payoutPreset: any;
}) {
  return {
    organizer: o.organizer,
    name: o.name,
    tokenMint: MINT,
    vault: o.vault,
    entryFee: new BN(ENTRY_FEE.toString()),
    organizerDeposit: new BN(0),
    organizerDepositRefunded: false,
    maxParticipants: 4,
    bracketSize: 2,
    participantCount: 2,
    matchesInitialized: 1,
    matchesReported: 0,
    totalMatches: 1,
    registrationDeadline: new BN(0),
    createdAt: new BN(0),
    startedAt: new BN(1_000_000),
    completedAt: new BN(0),
    status: { active: {} },
    payoutPreset: o.payoutPreset,
    seedHash: Array(32).fill(0),
    champion: PublicKey.default,
    bump: o.bump,
    vaultBump: o.vaultBump,
    game: { manual: {} },
    settlementMode: { playerReported: {} },
    disputeWindowSecs: 0,
    vrfRandomnessAccount: PublicKey.default,
    vrfCommitSlot: new BN(0),
    seedRevealed: true,
    arbitrator: o.arbitrator,
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
  playerA: PublicKey;
  playerB: PublicKey;
  bump: number;
}) {
  return {
    tournament: o.tournament,
    bracket: 0,
    round: 0,
    matchIndex: 0,
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
  buf.writeUInt8(1, 108);
  const lamports = svm.minimumBalanceForRentExemption(165n);
  svm.setAccount(
    pubkey.toBytes(),
    new LiteAccount(lamports, buf, TOKEN_PROGRAM.toBytes(), false, 0n)
  );
}
function tokenAmount(svm: LiteSvm, pubkey: PublicKey): bigint {
  const acc = svm.getAccount(pubkey.toBytes());
  return acc ? Buffer.from(acc.data()).readBigUInt64LE(64) : 0n;
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
function sendTx(svm: LiteSvm, ix: TransactionInstruction, signer: Keypair) {
  const tx = new Transaction().add(ix);
  tx.recentBlockhash = svm.latestBlockhash();
  tx.feePayer = signer.publicKey;
  tx.sign(signer);
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
function decode(
  svm: LiteSvm,
  name: "tournament" | "matchNode",
  pubkey: PublicKey
): any {
  const acc = svm.getAccount(pubkey.toBytes())!;
  return coder.accounts.decode(name, Buffer.from(acc.data()));
}
function funded(svm: LiteSvm): Keypair {
  const kp = new Keypair();
  svm.airdrop(kp.publicKey.toBytes(), 10n ** 9n);
  return kp;
}

// Pre-seed a started PlayerReported tournament whose only match is the final,
// already proposed (champion = player_a). Returns everything finalize needs.
async function seedFinal(svm: LiteSvm, name: string, payoutPreset: any) {
  const organizer = funded(svm);
  const treasury = new Keypair().publicKey;
  const champion = funded(svm);
  const runnerUp = funded(svm);
  const [tournament, bump] = tournamentPda(organizer.publicKey, name);
  const [vault, vaultBump] = vaultPda(tournament);
  const [pcfg, pcfgBump] = protocolConfigPda();
  const [final, mBump] = matchPda(tournament, 0, 0);
  const [pA, pABump] = participantPda(tournament, champion.publicKey);
  const [pB, pBBump] = participantPda(tournament, runnerUp.publicKey);

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
      arbitrator: organizer.publicKey,
      name,
      vault,
      bump,
      vaultBump,
      payoutPreset,
    })
  );
  await writeAccount(
    svm,
    final,
    "matchNode",
    makeMatch({
      tournament,
      playerA: champion.publicKey,
      playerB: runnerUp.publicKey,
      bump: mBump,
    })
  );
  await writeAccount(
    svm,
    pA,
    "participant",
    makeParticipant({ tournament, wallet: champion.publicKey, bump: pABump })
  );
  await writeAccount(
    svm,
    pB,
    "participant",
    makeParticipant({ tournament, wallet: runnerUp.publicKey, bump: pBBump })
  );
  writeTokenAccount(svm, vault, tournament, GROSS);

  const treasuryAta = new Keypair().publicKey;
  const champAta = new Keypair().publicKey;
  const runnerAta = new Keypair().publicKey;
  writeTokenAccount(svm, treasuryAta, treasury, 0n);
  writeTokenAccount(svm, champAta, champion.publicKey, 0n);
  writeTokenAccount(svm, runnerAta, runnerUp.publicKey, 0n);

  // Real proposal pins the trustless winner.
  expectOk(
    sendTx(
      svm,
      buildIx(
        "proposeResult",
        [
          meta(champion.publicKey, true, true),
          meta(tournament, false, false),
          meta(final, false, true),
        ],
        { proposedWinner: champion.publicKey }
      ),
      champion
    )
  );

  return {
    organizer,
    treasury,
    champion,
    runnerUp,
    tournament,
    vault,
    pcfg,
    final,
    participantA: pA,
    participantB: pB,
    treasuryAta,
    champAta,
    runnerAta,
  };
}

// finalize account tail shared by claim_result / settle_final (signer differs).
function finalizeKeys(signer: PublicKey, ctx: any): AccountMeta[] {
  return [
    meta(signer, true, true),
    meta(ctx.tournament, false, true),
    meta(ctx.final, false, true),
    meta(PROGRAM_ID, false, true), // next_match = None (final)
    meta(ctx.participantA, false, true),
    meta(ctx.participantB, false, true),
    meta(ctx.pcfg, false, false),
    meta(ctx.vault, false, true),
    meta(PROGRAM_ID, false, true), // organizer_token_account = None
    meta(TOKEN_PROGRAM, false, false),
  ];
}

describe("settle_final / H-1 multi-placement final hardening (LiteSVM)", function () {
  it("blocks a permissionless multi-placement final, then settles it via the arbitrator", async () => {
    const svm = bootSvm();
    const ctx = await seedFinal(svm, "h1-standard", { standard: {} });
    const champion = ctx.champion.publicKey;
    const runnerUp = ctx.runnerUp.publicKey;

    const third = new Keypair().publicKey; // a semifinal loser the arbitrator adjudicates
    const thirdAta = new Keypair().publicKey;
    writeTokenAccount(svm, thirdAta, third, 0n);
    const attacker = funded(svm);
    const attackerAta = new Keypair().publicKey;
    writeTokenAccount(svm, attackerAta, attacker.publicKey, 0n);

    // NEG 1 — theft vector: a cranker claims the Standard final, redirecting
    // 3rd place to an attacker wallet. Rejected by the finalize gate.
    const cranker = funded(svm);
    const stolen = [
      ...finalizeKeys(cranker.publicKey, ctx),
      meta(ctx.champAta, false, true),
      meta(ctx.runnerAta, false, true),
      meta(attackerAta, false, true),
      meta(ctx.treasuryAta, false, true),
    ];
    expectCustomErr(
      sendTx(
        svm,
        buildIx("claimResult", stolen, {
          placements: [champion, runnerUp, attacker.publicKey],
        }),
        cranker
      ),
      ERR_UNTRUSTED_MULTI_PLACEMENT_FINAL
    );

    // NEG 2 — a non-arbitrator cannot settle_final (address constraint).
    const correct = [
      meta(ctx.champAta, false, true),
      meta(ctx.runnerAta, false, true),
      meta(thirdAta, false, true),
      meta(ctx.treasuryAta, false, true),
    ];
    expectCustomErr(
      sendTx(
        svm,
        buildIx(
          "settleFinal",
          [...finalizeKeys(attacker.publicKey, ctx), ...correct],
          { placements: [champion, runnerUp, third] }
        ),
        attacker
      ),
      ERR_UNAUTHORIZED
    );

    // POS — the arbitrator (= organizer) settles with adjudicated placements.
    // Standard 60/25/15 over the net pool; 3.5% fee to treasury.
    expectOk(
      sendTx(
        svm,
        buildIx(
          "settleFinal",
          [...finalizeKeys(ctx.organizer.publicKey, ctx), ...correct],
          { placements: [champion, runnerUp, third] }
        ),
        ctx.organizer
      )
    );

    const fee = (GROSS * FEE_BPS) / 10_000n;
    const net = GROSS - fee;
    expect(tokenAmount(svm, ctx.champAta)).to.equal((net * 6000n) / 10_000n);
    expect(tokenAmount(svm, ctx.runnerAta)).to.equal((net * 2500n) / 10_000n);
    expect(tokenAmount(svm, thirdAta)).to.equal((net * 1500n) / 10_000n);
    expect(tokenAmount(svm, ctx.treasuryAta)).to.equal(fee);

    const t = decode(svm, "tournament", ctx.tournament);
    expect(Object.keys(t.status)[0]).to.equal("completed");
    expect(t.champion.toBase58()).to.equal(champion.toBase58());
    expect(Object.keys(decode(svm, "matchNode", ctx.final).status)[0]).to.equal(
      "completed"
    );
  });

  it("still lets anyone permissionlessly claim a WinnerTakesAll final", async () => {
    const svm = bootSvm();
    const ctx = await seedFinal(svm, "h1-wta", { winnerTakesAll: {} });
    const champion = ctx.champion.publicKey;

    const cranker = funded(svm);
    const keys = [
      ...finalizeKeys(cranker.publicKey, ctx),
      meta(ctx.champAta, false, true),
      meta(ctx.treasuryAta, false, true),
    ];
    expectOk(
      sendTx(
        svm,
        buildIx("claimResult", keys, { placements: [champion] }),
        cranker
      )
    );

    const net = GROSS - (GROSS * FEE_BPS) / 10_000n;
    expect(tokenAmount(svm, ctx.champAta)).to.equal(net);
    expect(
      Object.keys(decode(svm, "tournament", ctx.tournament).status)[0]
    ).to.equal("completed");
  });
});
