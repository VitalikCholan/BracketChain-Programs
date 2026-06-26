#!/usr/bin/env node
// One-shot: call `migrate_protocol_config` on dev devnet to realloc the
// pre-V1.1 ProtocolConfig PDA up to the current INIT_SPACE. Authority signer
// is the default Solana CLI keypair (~/.config/solana/id.json), which must
// match `protocol_config.authority` on chain.
//
// Run once after deploying the migrate_protocol_config ix:
//   node scripts/migrate-protocol-config.mjs

import { readFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

import {
  createKeyPairSignerFromBytes,
  createSolanaRpc,
  createSolanaRpcSubscriptions,
  pipe,
  createTransactionMessage,
  setTransactionMessageFeePayerSigner,
  setTransactionMessageLifetimeUsingBlockhash,
  appendTransactionMessageInstructions,
  signTransactionMessageWithSigners,
  getSignatureFromTransaction,
  sendAndConfirmTransactionFactory,
} from "@solana/kit";

// SDK barrel doesn't re-export raw generated ix; import from the source dir
// directly. This works because the SDK exposes its src/ tree in the tarball
// (not just dist/), and `src/generated/` is plain TS that node loads as ESM.
// Fallback: hand-build the ix via @solana/kit + the discriminator.
import {
  getProgramDerivedAddress,
  getBytesEncoder,
  getAddressEncoder,
  AccountRole,
} from "@solana/kit";

// `propose_result_oracle` discriminator from the IDL — hardcoded so we don't
// pull the whole SDK src tree at runtime. (For migrate_protocol_config we
// derive at runtime from the function name, see below.)
const SYSTEM_PROGRAM = "11111111111111111111111111111111";
const PROTOCOL_CONFIG_SEED = new TextEncoder().encode("protocol_config");

// Anchor discriminator = first 8 bytes of sha256("global:<snake_case_name>").
async function anchorDiscriminator(name) {
  const data = new TextEncoder().encode(`global:${name}`);
  const hash = await crypto.subtle.digest("SHA-256", data);
  return new Uint8Array(hash).slice(0, 8);
}

async function findProtocolConfigPda(programId) {
  const [pda] = await getProgramDerivedAddress({
    programAddress: programId,
    seeds: [PROTOCOL_CONFIG_SEED],
  });
  return pda;
}

async function buildMigrateIx(programId, authority) {
  const protocolConfig = await findProtocolConfigPda(programId);
  const discriminator = await anchorDiscriminator("migrate_protocol_config");
  return {
    programAddress: programId,
    accounts: [
      { address: authority.address, role: AccountRole.WRITABLE_SIGNER, signer: authority },
      { address: protocolConfig, role: AccountRole.WRITABLE },
      { address: SYSTEM_PROGRAM, role: AccountRole.READONLY },
    ],
    data: discriminator,
  };
}

// Default to the public devnet RPC — fine for a single-tx admin script.
// Override with `RPC_URL=https://… node scripts/migrate-protocol-config.mjs`
// if you want to use a private endpoint (don't commit keys).
const RPC = process.env.RPC_URL ?? "https://api.devnet.solana.com";
const WS = (process.env.RPC_WS_URL ?? RPC).replace(/^http/, "ws");
const KEYPAIR_PATH = join(homedir(), ".config/solana/id.json");
const PROGRAM_ID = "BeTbkzJ5MPiZP9PZ2xnhsjXCyBxVasuZJwuLQEpGiovw";

async function main() {
  const secret = new Uint8Array(JSON.parse(readFileSync(KEYPAIR_PATH, "utf8")));
  const signer = await createKeyPairSignerFromBytes(secret);
  console.log("authority:", signer.address);

  const rpc = createSolanaRpc(RPC);
  const rpcSubs = createSolanaRpcSubscriptions(WS);

  const ix = await buildMigrateIx(PROGRAM_ID, signer);

  const { value: blockhash } = await rpc.getLatestBlockhash().send();

  const message = pipe(
    createTransactionMessage({ version: 0 }),
    (m) => setTransactionMessageFeePayerSigner(signer, m),
    (m) => setTransactionMessageLifetimeUsingBlockhash(blockhash, m),
    (m) => appendTransactionMessageInstructions([ix], m),
  );

  const signedTx = await signTransactionMessageWithSigners(message);
  const sig = getSignatureFromTransaction(signedTx);
  console.log("submitting tx", sig);

  const sendAndConfirm = sendAndConfirmTransactionFactory({ rpc, rpcSubscriptions: rpcSubs });
  await sendAndConfirm(signedTx, { commitment: "confirmed" });
  console.log("✓ migrated. tx:", sig);
}

main().catch((err) => {
  console.error("FAIL:", err.message);
  if (err.cause) console.error("cause:", err.cause);
  process.exit(1);
});
