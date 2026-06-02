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
// Stage B — player-reported settlement, LiteSVM tier (down-migrated from the
// validator suite). Drives the real propose/confirm/dispute/claim/resolve
// instructions against pre-seeded match state. SPL Token is a LiteSVM built-in,
// so final-match prize distribution runs here without a validator.
//
//   1. propose + confirm → advances the bracket + credits stats (non-final).
//   2. confirm of the final → distributes the prize pool (WTA).
//   3. propose → dispute → organizer resolveDispute (override winner).
//   4. propose → permissionless claim_result past the (zero) window.
//   5. guards: SettlementModeMismatch / NotCounterparty / ClaimWindowNotElapsed
//      / ProposalDisputed / early force-claim.
// ─────────────────────────────────────────────────────────────────────────────

const PROGRAM_ID = new PublicKey("3YpkUKBh8288XN2dCKSwBnEdyc5UozSJ19A1ZCLpUZsZ");
const TOKEN_PROGRAM = new PublicKey("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");

const ENTRY_FEE = 1_000_000n;
const FEE_BPS = 350n;
const MINT = new PublicKey("Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB");

const ERR_SETTLEMENT_MODE_MISMATCH = 6032;
const ERR_NOT_COUNTERPARTY = 6034;
const ERR_CLAIM_WINDOW_NOT_ELAPSED = 6038;
const ERR_PROPOSAL_DISPUTED = 6039;

const stubProvider: any = { connection: {}, publicKey: PublicKey.default };
const program = new Program<BracketChain>(idl as any, stubProvider);
const coder = program.coder;

// ── PDAs ──────────────────────────────────────────────────────────────────────
function tournamentPda(organizer: PublicKey, name: string): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("tournament"), organizer.toBuffer(), Buffer.from(name)], PROGRAM_ID);
}
function vaultPda(tournament: PublicKey): [PublicKey, number] {
  return PublicKey.findProgramAddressSync([Buffer.from("vault"), tournament.toBuffer()], PROGRAM_ID);
}
function participantPda(tournament: PublicKey, wallet: PublicKey): [PublicKey, number] {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("participant"), tournament.toBuffer(), wallet.toBuffer()], PROGRAM_ID);
}
function protocolConfigPda(): [PublicKey, number] {
  return PublicKey.findProgramAddressSync([Buffer.from("protocol_config")], PROGRAM_ID);
}
function matchPda(tournament: PublicKey, round: number, idx: number): [PublicKey, number] {
  const idxLe = Buffer.alloc(2);
  idxLe.writeUInt16LE(idx, 0);
  return PublicKey.findProgramAddressSync(
    [Buffer.from("match"), tournament.toBuffer(), Buffer.from([0]), Buffer.from([round]), idxLe], PROGRAM_ID);
}

// ── Fixtures ────────────────────────────────────────────────────────────────
function makeTournament(o: {
  organizer: PublicKey; name: string; vault: PublicKey; bump: number; vaultBump: number;
  status: any; settlementMode: any; disputeWindowSecs: number;
  bracketSize: number; participantCount: number; matchesInitialized: number;
  matchesReported: number; totalMatches: number;
}) {
  return {
    organizer: o.organizer, name: o.name, tokenMint: MINT, vault: o.vault,
    entryFee: new BN(ENTRY_FEE.toString()), organizerDeposit: new BN(0),
    organizerDepositRefunded: false, maxParticipants: o.bracketSize, bracketSize: o.bracketSize,
    participantCount: o.participantCount, matchesInitialized: o.matchesInitialized,
    matchesReported: o.matchesReported, totalMatches: o.totalMatches,
    registrationDeadline: new BN(0), createdAt: new BN(0), startedAt: new BN(1_000_000),
    completedAt: new BN(0), status: o.status, payoutPreset: { winnerTakesAll: {} },
    seedHash: Array(32).fill(0), champion: PublicKey.default, bump: o.bump, vaultBump: o.vaultBump,
    game: { manual: {} }, settlementMode: o.settlementMode, disputeWindowSecs: o.disputeWindowSecs,
    vrfRandomnessAccount: PublicKey.default, vrfCommitSlot: new BN(0), seedRevealed: true,
    arbitrator: PublicKey.default,
  };
}
function makeParticipant(o: { tournament: PublicKey; wallet: PublicKey; bump: number }) {
  return {
    tournament: o.tournament, wallet: o.wallet, seedIndex: 0, refundPaid: false, bump: o.bump,
    identityHash: Array(32).fill(0), identityAttestation: PublicKey.default,
    wins: 0, losses: 0, pointsFor: 0, pointsAgainst: 0,
  };
}
function makeMatch(o: {
  tournament: PublicKey; round: number; matchIndex: number;
  playerA: PublicKey; playerB: PublicKey; bump: number; status?: any;
}) {
  return {
    tournament: o.tournament, bracket: 0, round: o.round, matchIndex: o.matchIndex,
    playerA: o.playerA, playerB: o.playerB, winner: PublicKey.default,
    status: o.status ?? { active: {} }, bye: false, bump: o.bump,
    proposalSource: { none: {} }, proposer: PublicKey.default, proposedWinner: PublicKey.default,
    proposedAt: new BN(0), claimDeadline: new BN(0), disputed: false, disputeReason: 0,
    commitment: null, switchboardFeed: PublicKey.default,
  };
}
function makeProtocolConfig(o: { authority: PublicKey; treasury: PublicKey; bump: number }) {
  return {
    authority: o.authority, treasury: o.treasury, defaultMint: MINT, feeBps: Number(FEE_BPS),
    bump: o.bump, sasCredential: PublicKey.default, sasSchemas: Array(5).fill(PublicKey.default),
    switchboardQueue: PublicKey.default, maxStaleSlots: 100, minOracleSamples: 5,
  };
}

// ── LiteSVM helpers ─────────────────────────────────────────────────────────
function bootSvm(): LiteSvm {
  const svm = new LiteSvm();
  svm.setSysvars();
  svm.addProgramFromFile(
    PROGRAM_ID.toBytes(),
    path.join(__dirname, "..", "..", "target", "deploy", "bracket_chain.so"));
  return svm;
}
async function writeAccount(
  svm: LiteSvm, pubkey: PublicKey,
  name: "tournament" | "participant" | "matchNode" | "protocolConfig", obj: any,
) {
  const data = await coder.accounts.encode(name, obj);
  const lamports = svm.minimumBalanceForRentExemption(BigInt(data.length));
  svm.setAccount(pubkey.toBytes(), new LiteAccount(lamports, data, PROGRAM_ID.toBytes(), false, 0n));
}
function writeTokenAccount(svm: LiteSvm, pubkey: PublicKey, owner: PublicKey, amount: bigint) {
  const buf = Buffer.alloc(165);
  MINT.toBuffer().copy(buf, 0);
  owner.toBuffer().copy(buf, 32);
  buf.writeBigUInt64LE(amount, 64);
  buf.writeUInt8(1, 108);
  const lamports = svm.minimumBalanceForRentExemption(165n);
  svm.setAccount(pubkey.toBytes(), new LiteAccount(lamports, buf, TOKEN_PROGRAM.toBytes(), false, 0n));
}
function tokenAmount(svm: LiteSvm, pubkey: PublicKey): bigint {
  const acc = svm.getAccount(pubkey.toBytes());
  return acc ? Buffer.from(acc.data()).readBigUInt64LE(64) : 0n;
}
function meta(pubkey: PublicKey, isSigner: boolean, isWritable: boolean): AccountMeta {
  return { pubkey, isSigner, isWritable };
}
function buildIx(name: string, keys: AccountMeta[], args: any = {}): TransactionInstruction {
  return new TransactionInstruction({ programId: PROGRAM_ID, keys, data: coder.instruction.encode(name, args) });
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
    throw new Error(`expected success, got failure:\n${res.meta().logs().join("\n")}`);
  }
}
function expectCustomErr(res: any, code: number) {
  if (!(res instanceof FailedTransactionMetadata)) throw new Error(`expected failure (custom ${code}), got success`);
  const e = res.err();
  if (!(e instanceof TransactionErrorInstructionError)) throw new Error(`expected InstructionError, got ${e.toString()}`);
  const inner = e.err();
  if (!(inner instanceof InstructionErrorCustom)) throw new Error(`expected Custom, got ${inner.toString()}`);
  expect(inner.code).to.equal(code);
}
function decode(svm: LiteSvm, name: "tournament" | "matchNode" | "participant", pubkey: PublicKey): any {
  const acc = svm.getAccount(pubkey.toBytes())!;
  return coder.accounts.decode(name, Buffer.from(acc.data()));
}

// A funded keypair, registered as a Keypair so we can sign for it.
function funded(svm: LiteSvm): Keypair {
  const kp = new Keypair();
  svm.airdrop(kp.publicKey.toBytes(), 10n ** 9n);
  return kp;
}

interface Bracket {
  organizer: Keypair; tournament: PublicKey; vault: PublicKey; pcfg: PublicKey;
  treasury: PublicKey; treasuryAta: PublicKey;
}

// Seed a tournament + protocol config + funded vault. Matches/participants are
// added per-test.
async function seedTournament(svm: LiteSvm, o: {
  name: string; settlementMode: any; disputeWindowSecs: number;
  bracketSize: number; vaultAmount: bigint;
}): Promise<Bracket> {
  const organizer = funded(svm);
  const treasury = new Keypair().publicKey;
  const [tournament, bump] = tournamentPda(organizer.publicKey, o.name);
  const [vault, vaultBump] = vaultPda(tournament);
  const [pcfg, pcfgBump] = protocolConfigPda();
  await writeAccount(svm, pcfg, "protocolConfig", makeProtocolConfig({ authority: organizer.publicKey, treasury, bump: pcfgBump }));
  await writeAccount(svm, tournament, "tournament", makeTournament({
    organizer: organizer.publicKey, name: o.name, vault, bump, vaultBump,
    status: { active: {} }, settlementMode: o.settlementMode, disputeWindowSecs: o.disputeWindowSecs,
    bracketSize: o.bracketSize, participantCount: o.bracketSize,
    matchesInitialized: o.bracketSize - 1, matchesReported: 0, totalMatches: o.bracketSize - 1,
  }));
  writeTokenAccount(svm, vault, tournament, o.vaultAmount);
  const treasuryAta = new Keypair().publicKey;
  writeTokenAccount(svm, treasuryAta, treasury, 0n);
  return { organizer, tournament, vault, pcfg, treasury, treasuryAta };
}

// Seed a match + both participant PDAs; returns their addresses.
async function seedMatch(svm: LiteSvm, tournament: PublicKey, round: number, idx: number, playerA: PublicKey, playerB: PublicKey) {
  const [match, mBump] = matchPda(tournament, round, idx);
  await writeAccount(svm, match, "matchNode", makeMatch({ tournament, round, matchIndex: idx, playerA, playerB, bump: mBump }));
  const [pA, pABump] = participantPda(tournament, playerA);
  const [pB, pBBump] = participantPda(tournament, playerB);
  await writeAccount(svm, pA, "participant", makeParticipant({ tournament, wallet: playerA, bump: pABump }));
  await writeAccount(svm, pB, "participant", makeParticipant({ tournament, wallet: playerB, bump: pBBump }));
  return { match, participantA: pA, participantB: pB };
}
async function seedEmptyMatch(svm: LiteSvm, tournament: PublicKey, round: number, idx: number) {
  const [match, mBump] = matchPda(tournament, round, idx);
  await writeAccount(svm, match, "matchNode", makeMatch({
    tournament, round, matchIndex: idx, playerA: PublicKey.default, playerB: PublicKey.default,
    bump: mBump, status: { pending: {} },
  }));
  return match;
}

function proposeIx(proposer: PublicKey, tournament: PublicKey, match: PublicKey, winner: PublicKey) {
  return buildIx("proposeResult", [
    meta(proposer, true, true), meta(tournament, false, false), meta(match, false, true),
  ], { proposedWinner: winner });
}
function disputeIx(disputer: PublicKey, tournament: PublicKey, match: PublicKey, reason: number) {
  return buildIx("disputeResult", [
    meta(disputer, true, true), meta(tournament, false, false), meta(match, false, true),
  ], { disputeReason: reason });
}
// Shared finalize tail for confirm/claim/resolve.
function finalizeKeys(signer: PublicKey, b: Bracket, match: PublicKey, nextMatch: PublicKey | null,
  pA: PublicKey, pB: PublicKey, organizerAta: PublicKey | null): AccountMeta[] {
  return [
    meta(signer, true, true),
    meta(b.tournament, false, true),
    meta(match, false, true),
    meta(nextMatch ?? PROGRAM_ID, false, true),
    meta(pA, false, true),
    meta(pB, false, true),
    meta(b.pcfg, false, false),
    meta(b.vault, false, true),
    meta(organizerAta ?? PROGRAM_ID, false, true),
    meta(TOKEN_PROGRAM, false, false),
  ];
}

describe("player-reported settlement (LiteSVM)", function () {
  // ── 1. propose + confirm → advance + stats ──────────────────────────────────
  it("propose+confirm advances the bracket and credits stats (non-final)", async () => {
    const svm = bootSvm();
    const b = await seedTournament(svm, { name: "pr-happy", settlementMode: { playerReported: {} }, disputeWindowSecs: 60, bracketSize: 4, vaultAmount: 4n * ENTRY_FEE });
    const A = funded(svm), B = funded(svm);
    const { match: semi, participantA, participantB } = await seedMatch(svm, b.tournament, 0, 0, A.publicKey, B.publicKey);
    const final = await seedEmptyMatch(svm, b.tournament, 1, 0);

    expectOk(sendTx(svm, proposeIx(A.publicKey, b.tournament, semi, A.publicKey), A));
    expectOk(sendTx(svm, buildIx("confirmResult", finalizeKeys(B.publicKey, b, semi, final, participantA, participantB, null), { placements: [] }), B));

    const m = decode(svm, "matchNode", semi);
    expect(Object.keys(m.status)[0]).to.equal("completed");
    expect(m.winner.toBase58()).to.equal(A.publicKey.toBase58());
    expect(decode(svm, "participant", participantA).wins).to.equal(1);
    expect(decode(svm, "participant", participantB).losses).to.equal(1);
    expect(decode(svm, "matchNode", final).playerA.toBase58()).to.equal(A.publicKey.toBase58());
  });

  // ── 2. confirm of the final → distributes prizes (WTA) ──────────────────────
  it("confirm of the final distributes the prize pool", async () => {
    const svm = bootSvm();
    const b = await seedTournament(svm, { name: "pr-final", settlementMode: { playerReported: {} }, disputeWindowSecs: 60, bracketSize: 2, vaultAmount: 2n * ENTRY_FEE });
    const champ = funded(svm), loser = funded(svm);
    const { match: final, participantA, participantB } = await seedMatch(svm, b.tournament, 0, 0, champ.publicKey, loser.publicKey);
    const champAta = new Keypair().publicKey;
    writeTokenAccount(svm, champAta, champ.publicKey, 0n);

    expectOk(sendTx(svm, proposeIx(champ.publicKey, b.tournament, final, champ.publicKey), champ));
    const keys = finalizeKeys(loser.publicKey, b, final, null, participantA, participantB, null);
    keys.push(meta(champAta, false, true), meta(b.treasuryAta, false, true)); // remaining: placement + treasury
    expectOk(sendTx(svm, buildIx("confirmResult", keys, { placements: [champ.publicKey] }), loser));

    const basis = 2n * ENTRY_FEE;
    const fee = (basis * FEE_BPS) / 10_000n;
    expect(tokenAmount(svm, champAta)).to.equal(basis - fee);
    expect(tokenAmount(svm, b.treasuryAta)).to.equal(fee);
    expect(tokenAmount(svm, b.vault)).to.equal(0n);
    const t = decode(svm, "tournament", b.tournament);
    expect(Object.keys(t.status)[0]).to.equal("completed");
    expect(t.champion.toBase58()).to.equal(champ.publicKey.toBase58());
  });

  // ── 3. dispute → organizer resolveDispute (override) ────────────────────────
  it("dispute routes to the organizer, who resolves with the override winner", async () => {
    const svm = bootSvm();
    const b = await seedTournament(svm, { name: "pr-dispute", settlementMode: { playerReported: {} }, disputeWindowSecs: 60, bracketSize: 4, vaultAmount: 4n * ENTRY_FEE });
    const A = funded(svm), B = funded(svm);
    const { match: semi, participantA, participantB } = await seedMatch(svm, b.tournament, 0, 0, A.publicKey, B.publicKey);
    const next = await seedEmptyMatch(svm, b.tournament, 1, 0);

    expectOk(sendTx(svm, proposeIx(A.publicKey, b.tournament, semi, A.publicKey), A));
    expectOk(sendTx(svm, disputeIx(B.publicKey, b.tournament, semi, 1), B));
    const d = decode(svm, "matchNode", semi);
    expect(d.disputed).to.equal(true);
    expect(d.disputeReason).to.equal(1);

    // Organizer overrides: B wins.
    expectOk(sendTx(svm, buildIx("resolveDispute", finalizeKeys(b.organizer.publicKey, b, semi, next, participantA, participantB, null), { winner: B.publicKey, placements: [] }), b.organizer));
    const r = decode(svm, "matchNode", semi);
    expect(Object.keys(r.status)[0]).to.equal("completed");
    expect(r.winner.toBase58()).to.equal(B.publicKey.toBase58());
    expect(decode(svm, "participant", participantB).wins).to.equal(1);
  });

  // ── 4. permissionless claim past the (zero) window ──────────────────────────
  it("claim_result finalizes an undisputed proposal past the window", async () => {
    const svm = bootSvm();
    const b = await seedTournament(svm, { name: "pr-claim", settlementMode: { playerReported: {} }, disputeWindowSecs: 0, bracketSize: 4, vaultAmount: 4n * ENTRY_FEE });
    const A = funded(svm), B = funded(svm);
    const { match: semi, participantA, participantB } = await seedMatch(svm, b.tournament, 0, 0, A.publicKey, B.publicKey);
    const next = await seedEmptyMatch(svm, b.tournament, 1, 0);

    expectOk(sendTx(svm, proposeIx(A.publicKey, b.tournament, semi, A.publicKey), A));
    const cranker = funded(svm);
    expectOk(sendTx(svm, buildIx("claimResult", finalizeKeys(cranker.publicKey, b, semi, next, participantA, participantB, null), { placements: [] }), cranker));

    const m = decode(svm, "matchNode", semi);
    expect(Object.keys(m.status)[0]).to.equal("completed");
    expect(m.winner.toBase58()).to.equal(A.publicKey.toBase58());
  });

  // ── 5. guards ───────────────────────────────────────────────────────────────
  it("rejects propose on OrganizerOnly tournaments (SettlementModeMismatch)", async () => {
    const svm = bootSvm();
    const b = await seedTournament(svm, { name: "pr-guard-mode", settlementMode: { organizerOnly: {} }, disputeWindowSecs: 0, bracketSize: 4, vaultAmount: 4n * ENTRY_FEE });
    const A = funded(svm), B = funded(svm);
    const { match: semi } = await seedMatch(svm, b.tournament, 0, 0, A.publicKey, B.publicKey);
    expectCustomErr(sendTx(svm, proposeIx(A.publicKey, b.tournament, semi, A.publicKey), A), ERR_SETTLEMENT_MODE_MISMATCH);
  });

  it("rejects confirm by the proposer, claim before window, claim after dispute, and early force-claim", async () => {
    const svm = bootSvm();
    const b = await seedTournament(svm, { name: "pr-guards", settlementMode: { playerReported: {} }, disputeWindowSecs: 3600, bracketSize: 4, vaultAmount: 4n * ENTRY_FEE });
    const A = funded(svm), B = funded(svm);
    const { match: semi, participantA, participantB } = await seedMatch(svm, b.tournament, 0, 0, A.publicKey, B.publicKey);
    const next = await seedEmptyMatch(svm, b.tournament, 1, 0);
    expectOk(sendTx(svm, proposeIx(A.publicKey, b.tournament, semi, A.publicKey), A));

    // proposer cannot confirm their own proposal.
    expectCustomErr(sendTx(svm, buildIx("confirmResult", finalizeKeys(A.publicKey, b, semi, next, participantA, participantB, null), { placements: [] }), A), ERR_NOT_COUNTERPARTY);

    // claim before the (long) window elapses. Each attempt uses a distinct
    // signer so otherwise-identical txs don't collide on signature (duplicate).
    const cranker1 = funded(svm);
    expectCustomErr(sendTx(svm, buildIx("claimResult", finalizeKeys(cranker1.publicKey, b, semi, next, participantA, participantB, null), { placements: [] }), cranker1), ERR_CLAIM_WINDOW_NOT_ELAPSED);

    // dispute, then claim_result must refuse a disputed proposal.
    expectOk(sendTx(svm, disputeIx(B.publicKey, b.tournament, semi, 2), B));
    const cranker2 = funded(svm);
    expectCustomErr(sendTx(svm, buildIx("claimResult", finalizeKeys(cranker2.publicKey, b, semi, next, participantA, participantB, null), { placements: [] }), cranker2), ERR_PROPOSAL_DISPUTED);

    // force-claim is gated behind the re-armed 24h deadline → too early now.
    const cranker3 = funded(svm);
    expectCustomErr(sendTx(svm, buildIx("forceClaimDisputed", finalizeKeys(cranker3.publicKey, b, semi, next, participantA, participantB, null), { placements: [] }), cranker3), ERR_CLAIM_WINDOW_NOT_ELAPSED);
  });
});
