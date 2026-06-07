#!/usr/bin/env node
// One-shot: spin up a fully-active Oracle-mode tournament for C-10 UI testing.
//
//   1. Create tournament — settlementMode=Oracle, game=Manual (no SAS needed),
//      maxParticipants=2, entryFee=0.
//   2. Generate 2 fresh keypairs, airdrop each from devnet faucet.
//   3. Join both into the tournament.
//   4. Start tournament — flips Registration → Active with bracketSize=2, one
//      MatchNode initialized.
//
// After this, the frontend's `/t/<PDA>` page should render the C-10 surfaces
// with the CLI keypair connected as organizer:
//   - sidebar: ORG + ARB tags
//   - bracket: one match card with "Awaiting commit" pill
//   - click match → CommitAndBindPanel
//
// Run:  node scripts/seed-oracle-bracket.mjs

import { readFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

import {
  appendTransactionMessageInstructions,
  createKeyPairSignerFromBytes,
  createTransactionMessage,
  generateKeyPairSigner,
  getSignatureFromTransaction,
  lamports,
  pipe,
  sendAndConfirmTransactionFactory,
  setTransactionMessageFeePayerSigner,
  setTransactionMessageLifetimeUsingBlockhash,
  signTransactionMessageWithSigners,
} from "@solana/kit";
import { getTransferSolInstruction } from "@solana-program/system";

import {
  BracketChainClient,
  createTournament,
  joinTournament,
  startTournament,
  PayoutPreset,
  SettlementMode,
  SupportedGame,
} from "../../BracketChain-Sdk/dist/index.mjs";

const RPC = process.env.RPC_URL ?? "https://api.devnet.solana.com";
const WS = (process.env.RPC_WS_URL ?? RPC).replace(/^http/, "ws");
const KEYPAIR_PATH = join(homedir(), ".config/solana/id.json");
const PROGRAM_ID = "3YpkUKBh8288XN2dCKSwBnEdyc5UozSJ19A1ZCLpUZsZ";
const FRONTEND = process.env.FRONTEND_URL ?? "http://localhost:3000";

const NAME = process.env.ORACLE_TOURNEY_NAME ?? `oracle-active-${Math.floor(Date.now() / 1000)}`;

function makeClient(signer) {
  return new BracketChainClient({
    rpc: RPC,
    rpcSubscriptions: WS,
    signer,
    programAddress: PROGRAM_ID,
    commitment: "confirmed",
  });
}

async function main() {
  // ── organizer: CLI keypair ─────────────────────────────────────────────
  const secret = new Uint8Array(JSON.parse(readFileSync(KEYPAIR_PATH, "utf8")));
  const organizer = await createKeyPairSignerFromBytes(secret);
  console.log("organizer:", organizer.address);

  const orgClient = makeClient(organizer);

  // ── 1. create Oracle tournament (Manual game → skips SAS) ──────────────
  const deadlineSec = BigInt(Math.floor(Date.now() / 1000) + 24 * 3600);
  console.log(`\n[1/4] create tournament "${NAME}" (Manual + Oracle + 2 players, no entry fee)`);
  const created = await createTournament(orgClient, {
    name: NAME,
    entryFee: 0n,
    maxParticipants: 2,
    payoutPreset: PayoutPreset.WinnerTakesAll,
    registrationDeadline: deadlineSec,
    game: SupportedGame.Manual,
    settlementMode: SettlementMode.Oracle,
  });
  const pda = created.tournamentPda;
  console.log("    tournamentPda:", pda);

  // ── 2. generate + airdrop 2 player keypairs ────────────────────────────
  console.log("\n[2/4] mint 2 player keypairs + airdrop");
  const playerA = await generateKeyPairSigner();
  const playerB = await generateKeyPairSigner();
  console.log("    playerA:", playerA.address);
  console.log("    playerB:", playerB.address);

  // 0.05 SOL each — enough for joinTournament + a small buffer for Match PDAs.
  // Public devnet airdrop is rate-limited; Helius requires a paid plan. Use a
  // direct SOL transfer from the organizer (who already holds devnet SOL).
  const sendAndConfirm = sendAndConfirmTransactionFactory({
    rpc: orgClient.rpc,
    rpcSubscriptions: orgClient.rpcSubscriptions,
  });
  const transferIxs = [playerA.address, playerB.address].map((to) =>
    getTransferSolInstruction({ source: organizer, destination: to, amount: lamports(50_000_000n) }),
  );
  const { value: blockhash } = await orgClient.rpc.getLatestBlockhash().send();
  const fundTx = pipe(
    createTransactionMessage({ version: 0 }),
    (m) => setTransactionMessageFeePayerSigner(organizer, m),
    (m) => setTransactionMessageLifetimeUsingBlockhash(blockhash, m),
    (m) => appendTransactionMessageInstructions(transferIxs, m),
  );
  const signedFund = await signTransactionMessageWithSigners(fundTx);
  await sendAndConfirm(signedFund, { commitment: "confirmed" });
  console.log(`    ✓ funded both players (0.05 SOL each) tx=${getSignatureFromTransaction(signedFund).slice(0, 8)}`);

  // ── 3. join both ───────────────────────────────────────────────────────
  console.log("\n[3/4] join both players");
  for (const signer of [playerA, playerB]) {
    const client = makeClient(signer);
    const { participantPda, txSignature } = await joinTournament(client, {
      tournamentPda: pda,
    });
    console.log(`    ✓ joined ${signer.address.slice(0, 6)}… participantPda=${participantPda.slice(0, 6)}… tx=${txSignature.slice(0, 8)}`);
  }

  // ── 4. start ───────────────────────────────────────────────────────────
  console.log("\n[4/4] start tournament");
  const start = await startTournament(orgClient, { tournamentPda: pda });
  console.log(`    ✓ bracketSize=${start.bracketSize} totalMatches=${start.totalMatches} chunks=${start.txSignatures.length}`);

  console.log("\n────────────────────────────────────────────");
  console.log("✓ Active Oracle tournament ready");
  console.log(`  open: ${FRONTEND}/t/${pda}`);
  console.log("  expect: ARB tag in sidebar, 1 match card with 'Awaiting commit' pill,");
  console.log("  click match → CommitAndBindPanel with lobby-id + copy + commit button");
}

main().catch((err) => {
  console.error("FAIL:", err.message);
  if (err.cause) console.error("cause:", err.cause.message ?? err.cause);
  process.exit(1);
});
