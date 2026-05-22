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
# Init script lives in the SDK repo because it shares IDL + PDA helpers with
# the published @bracketchain/sdk package — single source of truth.
#
# Requires: anchor CLI, pnpm, and a funded Solana keypair on the target cluster.

SDK_DIR := ../bracket-chain-sdk
INDEXER_DIR := ../bracket-chain-indexer
RPC_DEVNET ?= https://api.devnet.solana.com

# Toolchain: All targets assume a POSIX shell with anchor, node, npm on PATH.
# Windows users: run from inside WSL2 (Anchor + Solana CLI require Linux).

.PHONY: help build test codama-generate sync-idl-events sync-idl deploy-devnet init-devnet verify-devnet redeploy-devnet

help:
	@echo "BracketChain program recipes:"
	@echo "  make build              — anchor build + codama-generate + sync-idl-events"
	@echo "  make codama-generate    — regenerate Codama clients into SDK + indexer src/generated/"
	@echo "  make sync-idl-events    — copy raw IDL JSON to indexer (event-decoder fallback)"
	@echo "  make sync-idl           — deprecated alias for codama-generate + sync-idl-events"
	@echo "  make test               — anchor test (mocha against local validator)"
	@echo "  make deploy-devnet      — anchor deploy → init-protocol on devnet (idempotent)"
	@echo "  make init-devnet        — run init-protocol only (skip deploy)"
	@echo "  make verify-devnet      — fetch ProtocolConfig from devnet and print"
	@echo "  make redeploy-devnet    — alias for deploy-devnet (init re-runs idempotent)"
	@echo ""
	@echo "Override RPC: make deploy-devnet RPC_DEVNET=https://devnet.helius-rpc.com/?api-key=KEY"

# Build re-runs Codama so SDK + indexer generated/ trees never silently fall
# behind on event/account layout changes (was: raw IDL cp; now: typed regen).
build: anchor-build codama-generate sync-idl-events

anchor-build:
	anchor build

# Phase 0 (Stage 1): regenerate typed Codama clients into both consumer repos.
# Replaces the old hand-sync of vendored IDL JSON. SDK no longer needs the
# vendored IDL at all. Output: <consumer>/src/generated/src/generated/ —
# accounts/, instructions/, errors/, pdas/, programs/, types/.
codama-generate:
	npx codama run --all
	@echo "Codama clients regenerated → SDK + indexer src/generated/"

# Transitional (Phase 0 Stage 2): @codama/renderers-js v2.x does NOT emit
# events/ yet, so the indexer keeps a raw IDL JSON copy for BorshCoder event
# decoding only. Drop this target once renderer emits events/ or we hand-roll
# event codecs (see bracketchain-phase-0-foundation.md Stage 2 decision).
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
