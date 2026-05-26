# BracketChain-Programs build/codegen targets.
#
# Codama drives client generation for the SDK (single owner — F-0b).
# Run `make codama-generate` after any IDL-affecting program change, then
# commit the regenerated SDK tree alongside the program diff so the SDK
# builders stay in lockstep with on-chain accounts/ix. The indexer decodes
# events via its hand-typed BorshCoder parser + Prisma client, not Codama.

.PHONY: build idl codama-generate sync-idl

build:
	anchor build

# Refresh `target/idl/bracket_chain.json` only — used by `codama-generate`.
idl:
	anchor build

# Regenerate the Codama client into ../BracketChain-Sdk/src/generated (flat).
# NOTE: codama.json's first renderer arg is the PACKAGE FOLDER (where
# package.json lives) — the renderer writes flat to <folder>/src/generated.
# Pointing it at .../src/generated would nest to src/generated/src/generated.
codama-generate: idl
	npx codama run --all

# Legacy alias from pre-Codama days. Now routes through Codama.
sync-idl: codama-generate
	@echo "Deprecated: use 'make codama-generate' going forward."
