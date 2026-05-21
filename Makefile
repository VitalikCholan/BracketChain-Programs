# BracketChain-Programs build/codegen targets.
#
# Codama drives client generation for the sibling repos (SDK + Indexer).
# Run `make codama-generate` after any IDL-affecting program change, then
# commit the regenerated trees alongside the program diff so the indexer
# parser + SDK builders stay in lockstep with on-chain accounts/ix.

.PHONY: build idl codama-generate sync-idl

build:
	anchor build

# Refresh `target/idl/bracket_chain.json` only — used by `codama-generate`.
idl:
	anchor build

# Regenerate Codama clients into ../BracketChain-Sdk/src/generated AND
# ../BracketChain-Indexer/src/generated. Expects `codama.json` at the
# repo root + sibling repos checked out as peers.
codama-generate: idl
	npx codama run --all

# Legacy alias from pre-Codama days. Now routes through Codama.
sync-idl: codama-generate
	@echo "Deprecated: use 'make codama-generate' going forward."
