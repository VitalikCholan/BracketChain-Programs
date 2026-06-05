# BracketChain — program ops recipes.
#
# Why this file exists:
#   Solana programs deploy as bytecode only — there is no constructor. State
#   accounts (like ProtocolConfig) MUST be created via a separate `initialize_*`
#   instruction in a follow-up tx. Anchor's `migrations/deploy.ts` hook is
#   officially "early-stage" and unused by every serious protocol on Solana
#   (Drift v2, MarginFi v2, Squads v4, Quarry, Marinade — all use standalone
#   scripts). The deploy + init pair lives here so the bootstrap step can never
#   be forgotten when redeploying.
#
# Codama drives client generation for the SDK (single owner — F-0b).
# Run `make codama-generate` after any IDL-affecting program change, then
# commit the regenerated SDK tree alongside the program diff so the SDK
# builders stay in lockstep with on-chain accounts/ix. The indexer decodes
# events via its hand-typed BorshCoder parser + Prisma client, not Codama —
# it only consumes the raw IDL JSON copy (see `sync-idl-events`).
#
# Init script lives in the SDK repo because it shares IDL + PDA helpers with
# the published @bracketchain/sdk package — single source of truth.
#
# Requires: anchor CLI, pnpm, and a funded Solana keypair on the target cluster.

SDK_DIR := ../bracket-chain-sdk
INDEXER_DIR := ../bracket-chain-indexer
RPC_DEVNET ?= https://api.devnet.solana.com

# Toolchain: All targets assume a POSIX shell with anchor, node, npm on PATH.
# Windows users: run from inside WSL2 (Anchor + Solana CLI require Linux).

.PHONY: help build idl test codama-generate sync-idl-events sync-idl deploy-devnet init-devnet verify-devnet redeploy-devnet

help:
	@echo "BracketChain program recipes:"
	@echo "  make build              — anchor build + codama-generate + sync-idl-events"
	@echo "  make codama-generate    — regenerate Codama client into SDK src/generated/"
	@echo "  make sync-idl-events    — copy raw IDL JSON to indexer (event-decoder fallback)"
	@echo "  make sync-idl           — deprecated alias for codama-generate + sync-idl-events"
	@echo "  make test               — anchor test (mocha against local validator)"
	@echo "  make deploy-devnet      — anchor deploy → init-protocol on devnet (idempotent)"
	@echo "  make init-devnet        — run init-protocol only (skip deploy)"
	@echo "  make verify-devnet      — fetch ProtocolConfig from devnet and print"
	@echo "  make redeploy-devnet    — alias for deploy-devnet (init re-runs idempotent)"
	@echo ""
	@echo "Override RPC: make deploy-devnet RPC_DEVNET=https://devnet.helius-rpc.com/?api-key=KEY"

# Build re-runs Codama so the SDK generated/ tree never silently falls behind
# on event/account layout changes, and refreshes the indexer's raw IDL copy.
build: idl codama-generate sync-idl-events

# Refresh `target/idl/bracket_chain.json` — used by `codama-generate`.
idl:
	anchor build

# Regenerate the Codama client into ../BracketChain-Sdk/src/generated (flat).
# NOTE: codama.json's first renderer arg is the PACKAGE FOLDER (where
# package.json lives) — the renderer writes flat to <folder>/src/generated.
# Pointing it at .../src/generated would nest to src/generated/src/generated.
codama-generate: idl
	npx codama run --all
	@echo "Codama client regenerated → SDK src/generated/"

# The indexer keeps a raw IDL JSON copy for BorshCoder event decoding only
# (its own Codama adoption is deferred to indexer Phase 2).
sync-idl-events:
	cp target/idl/bracket_chain.json   $(INDEXER_DIR)/src/idl/bracket_chain.json
	@echo "Indexer IDL copy refreshed (event-decoder fallback)."

# Deprecated alias — preserved for muscle memory. Runs the two new targets.
sync-idl: codama-generate sync-idl-events
	@echo "⚠️  'sync-idl' is deprecated. Use 'make codama-generate' (or 'make build')."

test:
	anchor test

# Full bootstrap: upload bytecode, then ensure ProtocolConfig exists with
# canonical devnet USDC mint. Init is idempotent (skips if already initialized).
deploy-devnet:
	anchor deploy --provider.cluster devnet
	cd $(SDK_DIR) && pnpm tsx scripts/init-protocol.ts --rpc=$(RPC_DEVNET)

init-devnet:
	cd $(SDK_DIR) && pnpm tsx scripts/init-protocol.ts --rpc=$(RPC_DEVNET)

# Convenience read — confirm the singleton's authority/treasury/usdc_mint match expectations.
verify-devnet:
	cd $(SDK_DIR) && pnpm tsx scripts/init-protocol.ts --rpc=$(RPC_DEVNET)

redeploy-devnet: deploy-devnet
