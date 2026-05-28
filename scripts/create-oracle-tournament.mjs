#!/usr/bin/env node
// One-shot: create an Oracle-mode tournament on dev devnet via SDK.
//
// The frontend create-form ships no settlement picker (V1.2: settlementMode
// defaults to OrganizerOnly on every create), so this script is the only path
// to spawn an Oracle tournament for C-10 UI verification. Signs with the
// default Solana CLI keypair (~/.config/solana/id.json).
//
// Args (env overrides):
//   RPC_URL                  (default: https://api.devnet.solana.com)
//   ORACLE_TOURNEY_NAME      (default: oracle-test-<unix>)
//   ORACLE_MAX_PARTICIPANTS  (default: 2)
//   ORACLE_ENTRY_FEE_MICRO   (default: 0 — no-fee tournament; no USDC ATA needed)
//   FRONTEND_URL             (default: http://localhost:3000)

import { readFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

import { createKeyPairSignerFromBytes } from "@solana/kit";

import {
  BracketChainClient,
  createTournament,
  PayoutPreset,
  SettlementMode,
  SupportedGame,
} from "../../BracketChain-Sdk/dist/index.mjs";

const RPC = process.env.RPC_URL ?? "https://api.devnet.solana.com";
const WS = (process.env.RPC_WS_URL ?? RPC).replace(/^http/, "ws");
const KEYPAIR_PATH = join(homedir(), ".config/solana/id.json");
const PROGRAM_ID = "3YpkUKBh8288XN2dCKSwBnEdyc5UozSJ19A1ZCLpUZsZ";
const FRONTEND = process.env.FRONTEND_URL ?? "http://localhost:3000";

const NAME = process.env.ORACLE_TOURNEY_NAME ?? `oracle-${Math.floor(Date.now() / 1000)}`;
const MAX_PARTICIPANTS = Number(process.env.ORACLE_MAX_PARTICIPANTS ?? 2);
const ENTRY_FEE_MICRO = BigInt(process.env.ORACLE_ENTRY_FEE_MICRO ?? "0");

async function main() {
  const secret = new Uint8Array(JSON.parse(readFileSync(KEYPAIR_PATH, "utf8")));
  const signer = await createKeyPairSignerFromBytes(secret);
  console.log("organizer:", signer.address);

  const client = new BracketChainClient({
    rpc: RPC,
    rpcSubscriptions: WS,
    signer,
    programAddress: PROGRAM_ID,
    commitment: "confirmed",
  });

  // 2-player bracket → one match. WinnerTakesAll keeps the final-placements
  // path simple. Registration deadline 24h out so we have time to test.
  const deadlineSec = BigInt(Math.floor(Date.now() / 1000) + 24 * 3600);

  console.log(`creating: name="${NAME}" maxParticipants=${MAX_PARTICIPANTS} entryFee=${ENTRY_FEE_MICRO} settlementMode=Oracle`);

  const result = await createTournament(client, {
    name: NAME,
    entryFee: ENTRY_FEE_MICRO,
    maxParticipants: MAX_PARTICIPANTS,
    payoutPreset: PayoutPreset.WinnerTakesAll,
    registrationDeadline: deadlineSec,
    game: SupportedGame.Dota2,
    settlementMode: SettlementMode.Oracle,
  });

  console.log("\n✓ created");
  console.log("  tournamentPda:", result.tournamentPda);
  console.log("  vaultPda:     ", result.vaultPda);
  console.log("  tx:           ", result.txSignature);
  console.log("\nopen in browser:");
  console.log(`  ${FRONTEND}/t/${result.tournamentPda}`);
}

main().catch((err) => {
  console.error("FAIL:", err.message);
  if (err.cause) console.error("cause:", err.cause.message ?? err.cause);
  process.exit(1);
});
