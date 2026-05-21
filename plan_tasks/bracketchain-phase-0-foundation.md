# BracketChain Phase 0 — Pre-Phase-1 Foundation

> **Implementation playbook, not a design document.** Each section below is concrete steps with code samples. Phase 0 adopts industry-standard libraries (`solana-keychain`, `@codama/cli`) rather than spec'ing custom abstractions. Total effort: ~1.5 weeks solo, ~1 week team-of-2.

**Status:** ⏳ Not started. Earlier execution-log entries (dated 2026-05-18 / 2026-05-19) recorded a Codama integration attempt as "shipped" / "effectively complete," but a 2026-05-19 disk verification found that work absent: `bracket-chain-programs/codama.json` does not exist, `bracket-chain-sdk/src/generated/` does not exist, the SDK's `src/*` still imports from `@coral-xyz/anchor` (6 files), and the SDK's vendored `src/idl/bracket_chain.json` is still present. The indexer has an orphaned `src/generated/src/generated/` Codama tree, but no code path imports it (every `../generated/...` import resolves to `../generated/prisma`, which is Prisma's client, not Codama's output). **Phase 0 has not begun in any consumed code path.** The 2026-05-18 / 2026-05-19 execution-log sections below are preserved as planning notes — they describe what an integration attempt *would* look like — and are explicitly marked as not-shipped via a banner at the head of each.

Section 1 (`solana-keychain`) is still appropriately deferred to Phase 1 cron build for the reason originally stated — no MVP signing path exists to migrate today.

**Prerequisites:** MVP shipped on devnet (✅ done 2026-05-10), risk register entries R1 unresolved, R3 deferred to Phase 1.
**Last updated:** 2026-05-19 (rollback after disk verification).

---

## MVP context — what shipped (2026-05-10)

> Summary that replaces the deleted `bracketchain-mvp-plan.md`. Preserves the load-bearing facts (devnet identifiers, deploy/upgrade tx signatures, surface) for cross-plan reference. Design rationale, demo script, and submission-ceremony checklist are intentionally not preserved — they served their submission purpose and are not load-bearing for V1+. Original file lives in git history (`git log --all -- bracketchain-mvp-plan.md`) if forensic recovery is ever needed.

**Status.** All 7 MVP phases closed (Foundation → Smart Contracts → Phase 2.5 organizer deposit + multi-token rename + name-32 → Phase 2.6 `name` in `TournamentCreated` → Vertical slice → Lean indexer → SDK polish → UI polish → Submission). Repos tagged `v0.1.0-mvp` on 2026-05-10. SDK 0.3.1 published on npm (docs-polish patch over 0.3.0).

**Devnet anchors — treat as constants:**

| Item | Value |
|---|---|
| Program ID | `AuXJKpuZtkegs2ZSgopgckhN7Ev8bUz4zBc238LD2F1` |
| IDL account | `6hCNsRdgp9Rbw1saE29YUHDoXq7HHiqMec6omDX1JkBL` |
| Phase 2 deploy tx | `2NGiC7c9urakKLz6cugQyZpxb4D31wvYG7dFTs9DHv7jvQfzxWZ5xotGS2GcyMGrexWhctutcVPEnG6VfUuLhf9w` |
| Phase 2.5 upgrade tx (organizer deposit + multi-token rename + name≤32) | `3wtoomhdrFoGWemR3NQct8Y5BcAyXoL2QYPS9MWtxFuA1RXapD8aLve3tKKetSghrhCoU6o4otwH6cFnbhXtCJTR` |
| Phase 2.6 upgrade tx (`name` field in `TournamentCreated` event) | `3eViPHA99aVjLXjvfpbKacPbP1ic6J5VtjH3uiah3crTdqn4Y18Ehyx7uzbp7RpGZY1oVcnNVFboCsj8tyNdKiaN` |
| USDC devnet mint (advisory `default_mint` in `protocol_config`, not enforced per-tournament) | `4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU` |
| Frontend production URL | https://bracketchain.vercel.app |
| Indexer production URL | https://bracketchain-indexer-production.up.railway.app |

**Surface (canonical source = codebase; this is the cross-plan index):**

- **Program (Anchor 0.32.1):** 6 ix — `initialize_protocol`, `create_tournament`, `join_tournament`, `start_tournament`, `report_result`, `cancel_tournament`. 4 PDAs — `ProtocolConfig` `[b"protocol_config"]`, `Tournament` `[b"tournament", organizer, name]` (name ≤32 bytes), `Participant` `[b"participant", tournament, wallet]`, `MatchNode` `[b"match", tournament, round: u8, match_index_le_bytes(u16)]`. Vault PDA token account at `[b"vault", tournament]` with `token::authority = tournament` (NOT an ATA). 7 events — `TournamentCreated` (incl. `name`, `organizer_deposit`), `ParticipantRegistered`, `TournamentStarted`, `MatchReported`, `TournamentCompleted` (incl. `placement_payouts`, `treasury_recipient`), `TournamentCancelled`, `RefundIssued`. 3 payout presets — `WinnerTakesAll`, `Standard (60/25/15)`, `Deep (40/25/15/10/5/3/2)`. `[2, 128]` participants. 350 bps (3.5%) protocol fee. Status flow `Registration → PendingBracketInit → Active → Completed | Cancelled`. Single-elim only; no dispute window; pseudo-random seeding via `slot_hashes`; 1st+2nd validated on-chain, 3rd–Nth organizer-trusted; chunked `start_tournament` (7/chunk → 19 chunks for 128p) and chunked `cancel_tournament` (organizer flip + permissionless refund chunks).
- **Indexer (NestJS 11 + Prisma 7, Postgres on Neon, Railway):** 4 Prisma models — `Tournament`, `Participant`, `Match`, `Payout`. 1 SQL view — `protocol_fees`. Helius enhanced webhooks (`POST /webhooks/helius`, unauthenticated today — Section 3.1 closes this) + 60s reconciliation cron with slot-based freshness watermarks (`chainSlotAtWrite`). REST `/tournaments` (list, filterable by status), `/tournaments/:address`, `/tournaments/:address/{payouts,participants,matches}`, `/health` (incl. reconciliation diagnostics). BigInt fields serialize as decimal strings.
- **SDK (`@bracketchain/sdk@0.3.1`, TypeScript, tsup CJS+ESM):** Anchor-based, hand-written (Codama not yet adopted — see Section 2 of this doc). Two orthogonal client classes — `BracketChainClient` (program wrapper: tx construction, account deserialization, WebSocket account-subscriptions) + `BracketChainIndexerClient` (REST wrapper, AbortSignal-aware, fetch-based). `subscribe()` is single-pass `onAccountChange` + `onError` (no auto-resub; Drift v2 pattern deferred to V1 SDK hooks plan). 21 typed `BracketChainSDKError` subclasses + `mapError()` discriminator. Re-exports `BN` so consumers skip the `bn.js` install.
- **Frontend (Next.js 16 + React 19, Vercel):** Wallet-adapter (Phantom + Solflare via Wallet Standard); 6 routes — `/`, `/about`, `/create`, `/explore`, `/dashboard`, `/t/[id]`. SWR-style read pattern: indexer first (~150-slot freshness gate), RPC fallback when stale. No TanStack Query; custom `useReducer` per hook. Sonner toasts wired in `app/layout.tsx`. SDK 0.3.0 fully wired (organizer deposit, real wallet adapter, real error mapping → 11 typed branches surfacing as toasts + inline errors).
- **Architecture diagram & repo map:** `bracketchain-main/README.md`. On-chain surface above is also recoverable from `bracket-chain-programs/programs/bracket-chain/src/` (`lib.rs`, `state/*.rs`, `instructions/*.rs`, `events.rs`, `errors.rs`, `constants.rs`).

**Phase 5 acceptance gates (all closed at MVP submission, kept for cross-plan reference):**
- `/t/[address]` renders in <500ms from indexer (Phase 5.3).
- Stopping the indexer mid-session → frontend continues with RPC fallback, no errors (Phase 5.3 SWR).
- `subscribe()` fires within 1s of on-chain match-report tx confirmation.
- <5s sync latency under single webhook drop (Phase 5.4 reconcile cron <60s recovery).

**Known MVP gaps closed in Phase 0:** indexer webhook HMAC (Section 3.1), indexer test coverage (Section 3.2), name-check endpoint (Section 3.3), Tier-4 Anchor tests + CU baseline (Section 3.4), frontend README hygiene (Section 3.5).

**Known MVP gaps deferred to V1+:** VRF seeding (currently `slot_hashes`-derived; V1 player-reported plan owns); on-chain 3rd–Nth placement attestation (only 1st+2nd validated; V1+ candidate); Squads 2-of-3 multisig upgrade authority (mainnet-prep gate, not pre-Phase-1); multi-token wallet-balance UI (V1 webapp polish); SDK `subscribe()` auto-resub-on-disconnect, Drift v2 pattern (V1 SDK hooks plan); `@bracketchain/sdk/react` subpath publishing (V1 SDK hooks plan); `getProgramAccounts`-based participant/match reconciliation (only ships when webhook reliability becomes a real problem); `games` table for best-of-N series (V1 program redesign). See `bracketchain-roadmap.md` for sequencing.

---

## Why Phase 0 exists

Phase 1 redeploy bundle (V1.1 + V1 player-reported + V1.2 + program-improvements + partial-cancel) adds significant operational surface area:

- **5 new funded keypairs** for indexer-driven permissionless transactions.
- **9 new events** (with `event_version: u8` per V1.1 convention).
- **4 new cron services** scanning stalled on-chain state.

If Phase 1 starts without foundation work, each of these gets implemented **ad-hoc per module** — `process.env.X_KEYPAIR` reads scattered across cron services, vendored IDL copies manually synced, no defense against silent decode failures. Retrofit cost after Phase 1 is ~5-10× upfront cost, with regression risk on production-running code.

Phase 0 adopts three pieces of established Solana infrastructure to remove these failure modes **before** they materialize:

1. **`solana-keychain`** — Solana Foundation's unified signing library (covers R3).
2. **`@codama/cli`** — Solana Foundation's official client generator (covers R1).
3. **`event_version: u8` convention** — already locked in V1.1 plan's Scope decisions; applied at Phase 1 redeploy time, no Phase 0 work needed.

The third item is documented but **not executed in Phase 0** — it ships as part of V1.1's program code, not as a standalone task. Spelled out here so it's not mistaken for separate work.

**Section 3 added 2026-05-19** — five MVP gaps surfaced from cross-repo analysis that are cheaper to close pre-Phase-1 than after: unauthenticated indexer webhook (security), missing test coverage on the highest-risk indexer paths (regression risk multiplied by Phase 1's 9 new events), missing tournament-name-check endpoint (closes a frontend `TODO` and prevents opaque create-tx failures), missing CU baseline for the operations Phase 1 will grow (only way to detect Phase 1 CU regressions), and stale frontend README (R11). None of these are V1+ scope; all are pre-Phase-1 hygiene. See Section 3 for the per-item playbook.

---

## Acceptance gates

Phase 0 is **done** when all of these are true:

- [ ] All funded keypair usage in indexer goes through `keychain.signWith('role', tx)`. Zero direct `process.env.X_KEYPAIR` reads outside the keychain module. **(Section 1 — deferred to Phase 1 cron build 2026-05-19; MVP indexer has no signing path to migrate yet.)**
- [ ] `npx codama run --all` regenerates SDK + indexer clients from current MVP IDL. **Not started — `bracket-chain-programs/codama.json` does not exist on disk as of 2026-05-19. Earlier log claim ("Done 2026-05-18") not reflected in working tree.** When ready: the flag is `--all`, not `all` as written in Section 2.3 below.
- [ ] CI gate refuses any PR where committed generated client differs from `codama run` output. **Not started.** Cross-repo sibling checkout design problem remains unresolved; local regeneration discipline is the fallback.
- [ ] Existing MVP indexer's reconciliation cron still works (no behavior regression). **Vacuously true — Codama not adopted, so no regression risk introduced. When Stage 2 actually runs, this gate gets exercised for real.**
- [ ] Existing SDK consumers (frontend) still work (no API regression). **Vacuously true today — SDK still on `@coral-xyz/anchor` (6 files import it across `src/types.ts`, `src/methods/createTournament.ts`, `src/methods/joinTournament.ts`, `src/errors.ts`, `src/index.ts`, `src/client.ts`). Frontend continues consuming `@bracketchain/sdk@0.3.1` from npm (Anchor-based).** Becomes a real gate at Stage 4.
- [ ] `solana-keychain` adoption documented in indexer README. **(Section 1 deferred to Phase 1 cron build 2026-05-19.)**
- [ ] `@codama/cli` adoption documented in SDK + indexer + programs READMEs. **Not started — Codama not yet adopted in any consumed code path.**

### Section 3 — MVP gap closure gates

- [ ] **3.1** Indexer `POST /webhooks/helius` rejects unsigned requests with 401; HMAC guard wired; production secret rotated.
- [ ] **3.2** Indexer `pnpm test` covers all 7 event handlers (happy-path + re-delivery) and reconciliation drift cases; `scripts/test-parser.mjs` decodes a current Phase-2.5/2.6 payload (Task #20).
- [ ] **3.3** Indexer `GET /tournaments/check-name?organizer=...&name=...` ships; frontend `DuplicateNameWarning` mock replaced with real call.
- [ ] **3.4** Anchor suite green with new `organizer-deposit.test.ts` (3 tests) + `capacity-128p-deep.test.ts` (CU measurement); `CU_BUDGET.md` committed with baseline numbers; 128p Deep final-match payout under 1_400_000 CU.
- [ ] **3.5** `BracketChain-Frontend/README.md` accurately describes current state (R11 resolved).

If any gate fails, Phase 1 does not begin.

**Plan scope realization (2026-05-19 rollback):** The original Phase 0 plan treated Section 2 as a swap of vendored IDL for Codama-generated clients (~3-5 days). Earlier execution-log entries claimed this had begun and partially shipped, but a 2026-05-19 disk verification found no Codama integration in any consumed code path. The plan's *forward-looking analysis* still holds — Codama-generated clients are `@solana/kit`-style, a paradigm shift from `@coral-xyz/anchor`'s `Program` class — meaning when Phase 0 actually starts, it is properly understood as a multi-week SDK rewrite, not an import swap. Phase 0's effective scope, when started, is **4 stages**, not 2 sections:
1. **Stage 1** — Codegen pipeline (this plan's Section 2). ⏳ Not started.
2. **Stage 2** — Indexer parser migration to typed event interfaces. ⏳ Not started. Intended approach (Option A): keep BorshCoder for events, adopt Codama for accounts/types/instructions only — driven by `@codama/renderers-js@2.x` not emitting event decoders.
3. **Stage 3** — SDK rewrite onto `@solana/kit` + Codama builders. ⏳ Not started. SDK is still entirely on `@coral-xyz/anchor` (verified 2026-05-19).
4. **Stage 4** — Frontend boundary via `@solana/web3-compat`. ⏳ Not started.
5. **Section 1 (keychain)** — still applies but moved to "do during Phase 1 cron build" since no MVP signing path exists to migrate today.

---

## Section 1 — Adopt `solana-keychain` for funded keypair management

**Effort:** 1-2 days.
**Owner:** Indexer dev.
**Reference:** [Solana Production Readiness guide](https://solana.com/docs/payments/production-readiness) — recommends `solana-keychain` for production backend signing.

### 1.1. Install

In `bracket-chain-indexer/`:

```bash
pnpm add solana-keychain
```

Verify exact package name + version from current Solana Foundation docs at install time (library has evolved across releases).

### 1.2. Define named roles

Create `bracket-chain-indexer/src/keys/keychain.module.ts`:

```typescript
import { Module } from '@nestjs/common';
import { Keychain, MemoryBackend, AwsKmsBackend } from 'solana-keychain';

export type KeyRole =
  | 'claim-payer'      // Pays gas for claim_result auto-submission (V1 player-reported)
  | 'vrf-payer'        // Pays for Switchboard randomness requests (V1 player-reported)
  | 'refund-payer'     // Pays gas for partial_refund_chunk (V1 partial-cancel)
  | 'cleanup-payer'    // Pays gas for close_tournament chunks (V1 program-improvements)
  | 'sas-issuer';      // Signs SAS attestations (V1.1)

const backend = process.env.NODE_ENV === 'production'
  ? new AwsKmsBackend({ region: 'us-east-1', keyIdPrefix: 'bracketchain-' })
  : new MemoryBackend({ keysFromEnv: true });

export const keychain = new Keychain(backend, {
  roles: {
    'claim-payer':   { description: 'Pays gas for claim_result auto-submission' },
    'vrf-payer':     { description: 'Pays for Switchboard randomness requests' },
    'refund-payer':  { description: 'Pays gas for partial_refund_chunk' },
    'cleanup-payer': { description: 'Pays gas for close_tournament chunks' },
    'sas-issuer':    { description: 'Signs SAS attestations' },
  },
});

@Module({
  providers: [{ provide: 'KEYCHAIN', useValue: keychain }],
  exports: ['KEYCHAIN'],
})
export class KeychainModule {}
```

### 1.3. Migrate existing MVP code

Today's MVP indexer reads `process.env.INDEXER_PAYER_KEYPAIR` (or similar) for reconciliation. Migration:

```typescript
// BEFORE: scattered process.env reads
const payer = Keypair.fromSecretKey(bs58.decode(process.env.INDEXER_PAYER_KEYPAIR!));
const sig = await connection.sendTransaction(tx, [payer]);

// AFTER: keychain-mediated
constructor(@Inject('KEYCHAIN') private readonly keychain: Keychain) {}

await this.keychain.signWith('claim-payer', tx);
const sig = await connection.sendRawTransaction(tx.serialize());
```

For MVP, only one role exists today (reconciliation-related). Migrate it as a smoke test of the pattern. Phase 1 cron services will register additional roles.

### 1.4. Environment configuration

`.env.example` (development — keys loaded from base58-encoded env vars):

```bash
# Backend mode (development = MemoryBackend reads keys from env)
KEYCHAIN_BACKEND="memory"

# Per-role base58 secret keys (devnet)
KEYCHAIN_CLAIM_PAYER="base58-encoded-secret-key"
KEYCHAIN_VRF_PAYER="base58-encoded-secret-key"
KEYCHAIN_REFUND_PAYER="base58-encoded-secret-key"
KEYCHAIN_CLEANUP_PAYER="base58-encoded-secret-key"
KEYCHAIN_SAS_ISSUER="base58-encoded-secret-key"
```

For production (mainnet) — switch to AWS KMS:

```bash
KEYCHAIN_BACKEND="aws-kms"
AWS_REGION="us-east-1"
AWS_KMS_KEY_ID_PREFIX="bracketchain-prod-"
# Each role becomes bracketchain-prod-claim-payer, bracketchain-prod-vrf-payer, etc.
```

Backend swap requires zero code changes — `solana-keychain`'s entire value proposition.

### 1.5. Verification

```bash
# Smoke test: existing reconciliation cron still works
pnpm start:dev
# Wait 60 seconds for first cron tick
curl http://localhost:3000/health | jq '.reconciliation'
# Expect: lastReconcileAt populated, lastReconcileError null

# Audit: zero direct process.env keypair reads
grep -r "process.env.*KEYPAIR" src/ --exclude-dir=keys
# Expect: zero results
```

### 1.6. Section 1 acceptance gates

- [ ] `pnpm add solana-keychain` succeeds, package resolves.
- [ ] `KeychainModule` exports working `Keychain` instance with 5 roles defined.
- [ ] Existing MVP reconciliation cron migrated to `keychain.signWith()`; smoke-tests green.
- [ ] `grep` for `process.env.*KEYPAIR` outside `src/keys/` returns zero results.
- [ ] README updated with role catalog + backend-swap instructions.

---

## Section 2 — Codama pipeline for IDL-driven client generation

**Effort:** 3-5 days.
**Owner:** Combined SDK + indexer work.
**Reference:** [Generating Clients | Solana](https://solana.com/docs/programs/codama/clients) — official Codama documentation.

### 2.1. Install

In `bracket-chain-programs/`:

```bash
pnpm add -D @codama/cli @codama/nodes-from-anchor @codama/renderers-js
```

These are dev-deps because Codama runs at build time, not runtime.

### 2.2. Initialize configuration

```bash
cd bracket-chain-programs
npx codama init
```

Prompts:
- IDL location: `target/idl/bracket_chain.json`
- Output paths: configured manually after init (init creates single output; we need two)

Edit generated `codama.json`:

```json
{
  "idl": "target/idl/bracket_chain.json",
  "scripts": {
    "sdk": [
      {
        "from": "@codama/renderers-js",
        "args": ["../bracket-chain-sdk/src/generated"]
      }
    ],
    "indexer": [
      {
        "from": "@codama/renderers-js",
        "args": ["../bracket-chain-indexer/src/generated"]
      }
    ],
    "all": [
      {
        "from": "@codama/renderers-js",
        "args": ["../bracket-chain-sdk/src/generated"]
      },
      {
        "from": "@codama/renderers-js",
        "args": ["../bracket-chain-indexer/src/generated"]
      }
    ]
  }
}
```

### 2.3. Generate clients

```bash
npx codama run all
```

Verify `src/generated/` directory created in both SDK and indexer, containing:

```
src/generated/
├── accounts/       # Typed Tournament, MatchNode, Participant decoders
├── events/         # Typed event decoders (TournamentCreated, MatchReported, etc.)
├── instructions/   # Typed ix builders
├── types/          # Enums + structs
└── index.ts        # Re-exports
```

### 2.4. Migrate SDK imports

In `bracket-chain-sdk/src/index.ts` and downstream files:

```typescript
// BEFORE: vendored IDL + BorshCoder
import IDL from './idl/bracket_chain.json';
const coder = new BorshCoder(IDL);

// AFTER: generated decoders
export { decodeTournament, decodeMatchNode, decodeParticipant } from './generated/accounts';
export { decodeTournamentCreated, decodeMatchReported, ... } from './generated/events';
export { buildCreateTournamentIx, buildJoinTournamentIx, ... } from './generated/instructions';
```

Keep `src/idl/bracket_chain.json` for now as a fallback during migration; remove after all consumers migrated.

### 2.5. Migrate indexer parser

In `bracket-chain-indexer/src/webhooks/helius-parser.service.ts`:

```typescript
// BEFORE
import { BorshCoder, EventParser } from '@coral-xyz/anchor';
import IDL from '../idl/bracket_chain.json';
const eventParser = new EventParser(programId, new BorshCoder(IDL));

// AFTER
import { decodeEvent } from '../generated/events';

function parseEvent(logLine: string): DecodedEvent | null {
  const payload = extractEventPayload(logLine);
  return decodeEvent(payload);  // throws on version/shape mismatch
}
```

`decodeEvent` is the generated dispatcher; it inspects the discriminator + size and routes to the appropriate decoder, throwing on mismatch instead of silently writing garbage.

### 2.6. CI gate

In `bracket-chain-programs/.github/workflows/idl-check.yml`:

```yaml
name: Verify generated clients match IDL

on:
  pull_request:
    paths:
      - 'programs/**'
      - 'target/idl/**'
      - '../bracket-chain-sdk/src/generated/**'
      - '../bracket-chain-indexer/src/generated/**'

jobs:
  verify:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with:
          # Need siblings repos checked out
          submodules: recursive
      - uses: pnpm/action-setup@v2
      - run: pnpm install
      - name: Regenerate clients
        run: npx codama run all
      - name: Fail if generated output drifted
        run: |
          git diff --exit-code \
            ../bracket-chain-sdk/src/generated \
            ../bracket-chain-indexer/src/generated || \
            { echo "❌ Generated client out of sync. Run 'npx codama run all' and commit."; exit 1; }
```

This is **the load-bearing piece**. Without CI gate, Codama is just faster manual sync — still skippable. With CI gate, drift is structurally impossible.

### 2.7. Replace `make sync-idl`

Update `bracket-chain-programs/Makefile`:

```makefile
# BEFORE
sync-idl:
	cp target/idl/bracket_chain.json ../bracket-chain-sdk/src/idl/
	cp target/idl/bracket_chain.json ../bracket-chain-indexer/src/idl/

# AFTER
codama-generate:
	npx codama run all

# Keep sync-idl as alias for muscle memory, deprecated:
sync-idl: codama-generate
	@echo "⚠️  'sync-idl' is deprecated. Use 'make codama-generate' going forward."
```

### 2.8. Verification

```bash
# Smoke test: regeneration is deterministic
npx codama run all
git status  # should show no diff if generated correctly

# Smoke test: existing event handlers still decode correctly
# Replay a real Helius webhook payload from MVP tournament:
pnpm tsx scripts/test-parser.mjs <txSignature>
# Expect: events decoded with same field values as before migration

# Smoke test: CI gate fires on drift
# Manually edit src/generated/events/tournament-created.ts
# Push branch, open PR
# Expect: CI fails with "Generated client out of sync"
```

### 2.9. Section 2 acceptance gates

- [ ] ~~`npx codama init` and~~ `npx codama run --all` succeed. **Not started — `codama.json` does not exist in `bracket-chain-programs/` (verified 2026-05-19).** When running, skip the `codama init` interactive flow (Windows-Node ESM bug at config-load time) and hand-write `codama.json` directly. The plan's `npx codama run all` is incorrect — use `--all` flag.
- [ ] `src/generated/` populated in both SDK and indexer. **Not started in SDK; partial orphan in indexer.** SDK has no `src/generated/`. Indexer has `src/generated/src/generated/{accounts,errors,instructions,pdas,programs,types}/` from an earlier exploration, but no application code imports it (all `../generated/...` imports resolve to `../generated/prisma`, which is Prisma's client). When implementing, expect `@codama/renderers-js@2.x` package-wrapped layout (`src/generated/package.json` + `src/generated/src/generated/{accounts,instructions,errors,pdas,programs,types}/`), not the flat `accounts/`/`instructions/` tree this plan originally described.
- [ ] SDK consumers (frontend) work against generated decoders. **Not started.** Frontend consumes `@bracketchain/sdk@0.3.1` from npm; local SDK is also still on Anchor (not mid-rewrite as earlier log claimed).
- [ ] Indexer parser uses generated decoders. **Not started.** When implementing, expect Option A: Codama-generated decoders cover accounts/types/instructions, but `@codama/renderers-js@2.x` does NOT emit event log decoders, so `@coral-xyz/anchor`'s `EventParser` + `BorshCoder` remain for the 7 program events. The vendored IDL JSON stays in indexer (events-only, refreshed via `make sync-idl-events`).
- [ ] CI gate verified by manual drift test. **Not started** — cross-repo CI design unresolved; local regen discipline is the fallback.
- [~] `make sync-idl` deprecated, replaced by `make codama-generate`. **Makefile target exists but is broken** — `codama-generate` target calls `npx codama run --all` which fails because `codama.json` is missing. `sync-idl-events` target works (raw `cp` of IDL JSON to indexer).
- [ ] SDK + indexer + programs READMEs updated with Codama workflow. **Not started — no Codama workflow exists in any code path to document.**

---

## Section 3 — MVP gap closure (pre-Phase-1 hygiene)

**Effort:** ~3-5 days combined; parallelizable across indexer + programs.
**Owner:** Indexer dev (3.1, 3.2, 3.3, 3.5) + program dev (3.4).
**Why this section exists:** MVP shipped 2026-05-10 with five known gaps that the original Phase 0 plan didn't cover. They surfaced from cross-repo analysis on 2026-05-19. Closing them before the Phase 1 redeploy is materially cheaper than fixing them after — Phase 1 adds 9 new events, 4 new cron services, and the real-money entry flow (Steam-attested USDC tournaments via SAS + VRF + dispute resolution). Each unclosed gap compounds against that surface. Items explicitly **not** in this section because they remain V1+ scope: Squads 2-of-3 multisig upgrade authority (mainnet-prep gate, not pre-Phase-1), multi-token wallet-balance UI (V1 webapp polish), SDK `subscribe()` auto-resub Drift-v2 pattern (V1 SDK hooks plan), `@bracketchain/sdk/react` subpath (V1 SDK hooks plan), VRF seeding / on-chain 3rd-Nth placement attestation / `games` table (Phase 1 program work).

### 3.1. Indexer webhook HMAC authentication

**Effort:** 3-4 hours.

**Why now.** `POST /webhooks/helius` is unauthenticated today — `HELIUS_WEBHOOK_SECRET` is commented out in `bracket-chain-indexer/.env.example:24` with a "add HMAC guard before mainnet" note that was never wired. Anyone who learns the URL can POST fake event payloads and corrupt the DB (and via reconciliation cron, eventually mislead the frontend's SWR cache). Phase 1 pushes 9 new event types through the same endpoint and ships the first real-money flows; closing the guard before that lands means the new events arrive into an already-authenticated pipeline.

**Files:**
- `bracket-chain-indexer/src/webhooks/helius-hmac.guard.ts` (new) — NestJS `CanActivate` guard that reads `X-Helius-Signature` header and verifies HMAC-SHA256(raw_body, HELIUS_WEBHOOK_SECRET) using `crypto.timingSafeEqual` for constant-time compare.
- `bracket-chain-indexer/src/webhooks/webhooks.controller.ts` — `@UseGuards(HeliusHmacGuard)` on the POST handler.
- `bracket-chain-indexer/src/main.ts` — register `bodyParser.raw({ type: 'application/json' })` for the webhooks route so the guard sees raw bytes (NestJS's default JSON parser strips the signature-verifiable form).
- `bracket-chain-indexer/.env.example` — uncomment `HELIUS_WEBHOOK_SECRET=""` with a comment block explaining secret generation (`openssl rand -hex 32`).
- Helius dashboard — set the webhook's signing secret to the production value; document the rotation runbook in `bracket-chain-indexer/README.md`.

**Implementation note.** Helius webhook signing format has shifted across versions — verify the exact header name and HMAC envelope from current Helius docs at implementation time. If the header is `Authorization: Bearer <token>` rather than a signature, the guard becomes a shared-secret bearer check (less robust but supported by Helius's older webhook spec).

### 3.1 acceptance gates

- [ ] `curl -X POST $URL/webhooks/helius -d '{}'` without signature → 401.
- [ ] `curl -X POST $URL/webhooks/helius -d '<body>' -H "X-Helius-Signature: $(hmac body)"` with valid HMAC → 200.
- [ ] Production webhook secret rotated; replay of old captured payload (with old signature) → 401.
- [ ] `README.md` documents secret rotation runbook.

---

### 3.2. Indexer test coverage — webhook parser + reconciliation

**Effort:** 1-2 days.

**Why now.** Today's indexer test surface is `app.controller.spec.ts` + `app.e2e-spec.ts` — both NestJS scaffold "Hello World" stubs. The webhook parser is the highest-risk code path in the entire indexer (P6-4 in `bracketchain-mvp-plan.md` documents a production webhook drop that the reconciliation cron caught only after dedicated design effort). Phase 1 adds 9 new events into that parser, plus new reconciliation paths for Participant/Match drift. Shipping new event handlers into untested code, then watching them break on devnet (or worse, mainnet), wastes more time than writing the tests upfront.

**Test surface to add:**

| File | Coverage |
|---|---|
| `bracket-chain-indexer/src/webhooks/helius-parser.service.spec.ts` (new) | One happy-path test per event handler (7 events), one re-delivery idempotency test per event handler (7 events) — fixture payloads captured from devnet via `scripts/test-parser.mjs`. |
| `bracket-chain-indexer/src/reconciliation/reconciliation.service.spec.ts` (new) | Mock `ChainReaderService.fetchTournament` returning drift cases: status drift (e.g., DB says Active, chain says Completed), champion drift, slot drift only. Assert correct DB patch + correct emission of `lastReconcileAt` / `scanned` / `touched` health metrics. |
| `bracket-chain-indexer/scripts/test-parser.mjs` (fix) | Stale hardcoded payload predates Phase 2.5 token-mint rename. Capture a fresh post-2.5/2.6 webhook payload from devnet and replace the inline string. Tracked separately as Task #20 in this Phase 0 doc's outstanding-task list. |

**Implementation note.** Use Prisma's `prismaMock` pattern (`jest-mock-extended` + `DeepMockProxy<PrismaClient>`) for the parser tests rather than spinning a test Postgres — keeps test runtime under 5s, which matters when the suite gets re-run on every Phase 1 event-schema change.

### 3.2 acceptance gates

- [ ] `pnpm test` covers all 7 event handlers with happy-path + re-delivery test each.
- [ ] Reconciliation drift test passes for status, champion, slot drift cases.
- [ ] `scripts/test-parser.mjs` decodes a current-Phase-2.5/2.6 payload without throwing (Task #20 resolved).
- [ ] Test suite runs in under 10s wall-clock (cron-friendly for pre-commit hook adoption later).

---

### 3.3. Indexer name-check endpoint

**Effort:** 2-3 hours.

**Why now.** `BracketChain-Frontend/features/tournament/steps/DuplicateNameWarning.tsx:30` contains `TODO: replace with real API call` with a hardcoded mock list of tournament names. The frontend can't show accurate warnings until an indexer endpoint exists. Tournament name uniqueness is enforced by the PDA seed shape `[b"tournament", organizer, name.as_bytes()]` — collisions are *per-organizer*, not global. Without a check, users discover collisions only when their `create_tournament` tx fails with `AccountAlreadyInUse`, which surfaces as an opaque error in the wallet adapter, not as the readable warning the UI promises.

**Files:**
- `bracket-chain-indexer/src/tournaments/tournaments.controller.ts` — `GET /tournaments/check-name?organizer=<wallet>&name=<name>`.
- `bracket-chain-indexer/src/tournaments/tournaments.service.ts` — Prisma `findUnique({ where: { organizer_name: { organizer, name } } })` (composite unique key matches PDA seed semantics).
- Response shape: `{ taken: boolean, address?: string }` — `address` is the PDA when `taken: true`, useful for a "go to that tournament" link.
- **Frontend follow-up (out of Phase 0 scope but unblocked by it):** `BracketChain-Frontend/features/tournament/create/hooks/useNameCheck.ts` — debounced 300ms `useEffect` calling the endpoint, surfacing into `DuplicateNameWarning`. Frontend dev owns this once endpoint ships.

**Implementation note.** The endpoint can be public — name + organizer wallet are not sensitive data, and rate-limiting at the platform layer (Vercel/Railway) is enough. Don't add auth here; it slows the create-flow UX without security gain.

### 3.3 acceptance gates

- [ ] `GET /tournaments/check-name?organizer=<wallet>&name=<free>` returns `200 { taken: false }`.
- [ ] `GET /tournaments/check-name?organizer=<wallet>&name=<taken>` returns `200 { taken: true, address: "<pda>" }`.
- [ ] Frontend `DuplicateNameWarning` consumes the endpoint and warns on actual collision in the create wizard (frontend follow-up — verified at Phase 0 closeout, not gated on Phase 0 finish).

---

### 3.4. Tier-4 Anchor tests — organizer-deposit + 128p Deep CU validation

**Effort:** 1-2 days.
**Owner:** Program dev.

**Why now.** The MVP risk register (`bracketchain-mvp-plan.md` lines 213-218) names two CU-budget concerns: `report_result` final-match for Deep preset, and `cancel_tournament` with 128 participants. Both are Medium-likelihood / Medium-High-impact. The existing 128p test exercises bracket-init only — not the final-match payout path where the Deep preset CPI to 7 placement ATAs + treasury fee CPI live. The current 5 mocha tests all use `new BN(0)` for organizer deposit, so the entire Phase 2.5 deposit-refund + deposit-excluded-from-prize-pool path is untested.

Phase 1 redeploy grows MatchNode (proposal envelope: +83 bytes) and Participant (identity + stats: +74 bytes). Bigger account loads mean more CU per `getAccountInfo` and more compute in handlers that iterate accounts (chunked start, chunked cancel, final-match distribute). The only way to detect Phase 1 CU regressions is to establish the current MVP baseline before Phase 1 ships. Without a CU baseline, "did Phase 1 break the compute budget for 128p Deep payout?" is unanswerable — and finding out on mainnet is a redeploy.

**Tests to add:**

| File | Coverage |
|---|---|
| `bracket-chain-programs/tests/organizer-deposit.test.ts` (new) | Three tests: (1) `organizerDeposit > 0` refunds on pre-start cancel; (2) refund idempotency via `organizer_deposit_refunded` flag — calling cancel twice does not double-refund; (3) deposit excluded from prize-pool basis in `report_result` final-match — Deep payout percentages apply only to `vault_balance - organizer_deposit - protocol_fee`. |
| `bracket-chain-programs/tests/capacity-128p-deep.test.ts` (new) | 128 participants registered, Deep payout preset, full bracket reported to final. Validate CU consumption per ix using `confirmTransaction({ commitment, signature })` + `getTransaction(signature, { maxSupportedTransactionVersion: 0 })` reading `meta.computeUnitsConsumed`. Assert each ix stays under `1_400_000` CU. Final-match Deep payout is the most CU-intensive single ix. |
| `bracket-chain-programs/CU_BUDGET.md` (new) | Baseline numbers recorded: `create_tournament`, `join_tournament`, `start_tournament` (per-chunk), `report_result` (non-final), `report_result` (final, Deep, 128p), `cancel_tournament` (per-chunk, 128p). One row per preset where preset affects CU. Phase 1 redeploy ceremony re-runs this test and diffs against this file. |

**Implementation note.** Use `litesvm-mocha` or `solana-bankrun-mocha` if anchor's default validator is too slow at 128p scale. Setting up 128 keypairs + 128 ATAs + 128 token mints + 128 join txs takes >5 min on `solana-test-validator` even on fast hardware — bankrun runs the same suite in seconds. The existing 5 mocha tests can stay on `solana-test-validator`; the capacity test needs bankrun for iteration speed.

### 3.4 acceptance gates

- [ ] `anchor test` runs all 5 existing + 4 new tests green (or 5 existing + 4 new equivalents under bankrun).
- [ ] `CU_BUDGET.md` committed with baseline numbers for every ix.
- [ ] 128p Deep final-match payout completes under `1_400_000` CU.
- [ ] 128p chunked cancel completes within current chunk size (organizer-flip + 32-per-chunk refund).

---

### 3.5. Frontend README hygiene (R11)

**Effort:** 30 minutes.
**Owner:** Frontend-aware dev (no Solana program changes).

**Why now.** Current `BracketChain-Frontend/README.md` claims simulated transactions / pre-SDK-integration state. SDK 0.3.0 has been fully wired since 2026-05-10 (`[[project_frontend_state_2026_05_10]]`). New contributors reading the README get an inaccurate onboarding story — they think they're working with a UI shell, when they're actually working with a real wallet adapter + Sonner toasts + indexer SWR cache + Solana RPC fallback. Trivially cheap; closes R11 in the roadmap risk register.

**Files:**
- `BracketChain-Frontend/README.md` — rewrite the "Current state" / "What works" section. Mention: SDK 0.3.0 wiring, wallet adapter (Phantom + Solflare), Sonner toasts, indexer + RPC fallback, six live routes (`/`, `/about`, `/create`, `/explore`, `/dashboard`, `/t/[id]`), known TODOs (`DuplicateNameWarning` mock awaiting 3.3, custom-payout UI gated to MVP presets).

### 3.5 acceptance gates

- [ ] README accurately describes current state with no "simulated transaction" / "pre-SDK" claims.
- [ ] Setup instructions verified by running them on a clean checkout.

---

## Order of operations

> **Reality check 2026-05-19.** The day-counts below are the *plan-on-paper* estimates from when Phase 0 was scoped as 2 sections. The execution-log findings (preserved later in this doc) revealed the Codama→Kit migration is closer to **3-5 weeks solo** for Sections 2 alone. Section 3 adds **~3-5 days** on top. Treat the ordering below as the *sequence*, not the *duration* — and re-estimate against the 4-stage Codama scope when actually starting.

### Solo dev — Variant 1 (keychain first; Section 3 in parallel with Section 2)

```
Day 1-2:    Section 1 (solana-keychain) — DEFERRED to Phase 1
            (no MVP signing path to migrate today; lands when the first
             V1 signing cron is built — auto-claim / VRF-reveal / etc.)
Day 1:      Section 3.5 (frontend README) — 30 min, do first as a warm-up
Day 1-2:    Section 3.1 (webhook HMAC) — 3-4h
Day 3-4:    Section 3.3 (name-check endpoint) — 2-3h
            + Section 3.4 (Anchor Tier-4 tests) — start program-dev work in parallel
Day 5-6:    Section 3.2 (indexer test coverage) — 1-2 days
Day 7+:     Section 2 (Codama setup) — multi-week paradigm migration
            (Stage 1 codegen → Stage 2 indexer parser → Stage 3 SDK Kit
             rewrite → Stage 4 frontend boundary)
Day N:      Buffer + acceptance-gate verification across all sections
Day N+1:    Phase 1 program work begins
```

**Total: Section 3 ~3-5 days; Section 2 ~3-5 weeks (per execution-log revision); Section 1 deferred.**

### Team-of-2 — parallel execution

```
Dev 1 (Days 1-5):    Section 3.1 + 3.2 + 3.3 + 3.5 (indexer + frontend
                     hygiene) — all indexer-side or read-only frontend
Dev 2 (Days 1-5):    Section 3.4 (program Tier-4 tests + CU baseline)
                     + start Section 2 Stage 1 (Codama codegen pipeline)
Dev 2 (Week 2-3):    Section 2 Stages 2-4 (parser → SDK Kit → frontend bridge)
Dev 1 (Week 2-3):    Support Section 2 Stage 2 parser migration; Section 2
                     Stage 4 frontend boundary smoke-tests
Both (final day):    Combined acceptance-gate verification
Next day:            Phase 1 program work begins
```

**Total: ~3-4 weeks team-of-2 (down from ~1 week when Section 2 was scoped as import-swap).**

---

## Critical files (quick reference)

**Section 1 — keychain:**
- `bracket-chain-indexer/package.json` — add `solana-keychain` dep
- `bracket-chain-indexer/src/keys/keychain.module.ts` — new
- `bracket-chain-indexer/src/keys/keychain.module.spec.ts` — new (basic test)
- `bracket-chain-indexer/src/app.module.ts` — import `KeychainModule`
- `bracket-chain-indexer/src/reconciliation/reconciliation.service.ts` — migrate to `signWith`
- `bracket-chain-indexer/.env.example` — add `KEYCHAIN_*` env vars
- `bracket-chain-indexer/README.md` — document keychain pattern

**Section 2 — Codama:**
- `bracket-chain-programs/package.json` — add `@codama/cli` + renderers (dev-deps)
- `bracket-chain-programs/codama.json` — new (configuration)
- `bracket-chain-programs/Makefile` — replace `sync-idl` with `codama-generate`
- `bracket-chain-programs/.github/workflows/idl-check.yml` — new (CI gate)
- `bracket-chain-sdk/src/generated/` — new (generated; gitignore-checked)
- `bracket-chain-sdk/src/index.ts` — migrate exports to generated/
- `bracket-chain-indexer/src/generated/` — new (generated; gitignore-checked)
- `bracket-chain-indexer/src/webhooks/helius-parser.service.ts` — use generated decoders
- `bracket-chain-sdk/README.md` — document Codama workflow
- `bracket-chain-indexer/README.md` — document Codama workflow

**Section 3 — MVP gap closure:**
- `bracket-chain-indexer/src/webhooks/helius-hmac.guard.ts` — new (3.1)
- `bracket-chain-indexer/src/webhooks/webhooks.controller.ts` — `@UseGuards` (3.1)
- `bracket-chain-indexer/src/main.ts` — raw-body middleware (3.1)
- `bracket-chain-indexer/.env.example` — uncomment `HELIUS_WEBHOOK_SECRET` (3.1)
- `bracket-chain-indexer/src/webhooks/helius-parser.service.spec.ts` — new (3.2)
- `bracket-chain-indexer/src/reconciliation/reconciliation.service.spec.ts` — new (3.2)
- `bracket-chain-indexer/scripts/test-parser.mjs` — fix stale payload, Task #20 (3.2)
- `bracket-chain-indexer/src/tournaments/tournaments.controller.ts` — new `check-name` endpoint (3.3)
- `bracket-chain-indexer/src/tournaments/tournaments.service.ts` — `findUnique` query (3.3)
- `bracket-chain-programs/tests/organizer-deposit.test.ts` — new, 3 tests (3.4)
- `bracket-chain-programs/tests/capacity-128p-deep.test.ts` — new, CU measurement (3.4)
- `bracket-chain-programs/CU_BUDGET.md` — new, baseline file (3.4)
- `BracketChain-Frontend/README.md` — rewrite, R11 close (3.5)
- `BracketChain-Frontend/features/tournament/create/hooks/useNameCheck.ts` — new (3.3 follow-up, frontend-owned)
- `BracketChain-Frontend/features/tournament/steps/DuplicateNameWarning.tsx` — replace mock (3.3 follow-up)

**Docs:**
- `bracketchain-roadmap.md` — mark R1 + R3 + R11 as resolved when Phase 0 acceptance gates pass; record CU baseline in risk register notes

---

## Inspiration / reference protocols

These projects use the patterns Phase 0 adopts. Reference them when stuck on implementation details:

- **Codama in production:** [Solana Attestation Service](https://github.com/solana-foundation/solana-attestation-service) — uses Shank-based IDL + Codama-generated clients. CLAUDE.md explicitly notes "Automated generation pipeline ensures clients stay in sync."
- **`@solana/kit`:** 100% Codama-generated TypeScript SDK from Anchor. Battle-tested at scale.
- **Keypair management:** Solana Foundation Production Readiness guide endorses `solana-keychain` over ad-hoc env-var reads.
- **Helium env validation:** `helium/helium-program-library/packages/blockchain-api/src/lib/env.ts` — zod-schema-validated env vars (reference pattern even if you don't use zod).
- **Mango keeper:** `blockworks-foundation/mango-v4/bin/keeper/src/main.rs` — multi-interval single-binary keeper (relevant for Phase 1 `PermissionlessDriver` consolidation, which lands in Phase 1 work itself, not Phase 0).
- **Drift bot architecture:** [docs.drift.trade/developers/market-makers/bot-architecture](https://docs.drift.trade/developers/market-makers/bot-architecture) — mutex guards, slot-based cooldowns, watchdog timers. Use as inspiration for Phase 1 cron services.

---

## What Phase 0 does NOT include

Spelled out so it's not confused with later phases:

- ❌ `PermissionlessDriver` cron consolidation abstraction (R10) — lands in **Phase 1**, when the new cron services actually exist. Phase 0 keychain + Codama is foundation; cron consolidation is built on top during Phase 1 work.
- ❌ `event_version: u8` addition to existing MVP events — locked in V1.1 plan's Scope decisions; applies at Phase 1 redeploy time. No standalone MVP redeploy needed (would double migration ceremony for single byte).
- ❌ `IndexerKeyManager` from-scratch design — replaced by `solana-keychain` adoption (industry standard).
- ⚠️ Frontend changes — Phase 0 is primarily indexer + SDK + programs. **Exception (added 2026-05-19):** Section 3.5 touches `BracketChain-Frontend/README.md` (docs only, 30 min); Section 3.3 unblocks a small frontend follow-up (`useNameCheck` hook + `DuplicateNameWarning` mock removal) that is verified at Phase 0 closeout but owned by the frontend dev, not gated on the indexer endpoint shipping.
- ❌ New program ix or state changes — Phase 0 keeps current MVP program ID `AuXJKpuZtkegs2ZSgopgckhN7Ev8bUz4zBc238LD2F1` running unchanged. **Note:** Section 3.4 adds *tests* against the existing program, not new ix/state.
- ❌ Privy / Notifications / Codama-Rust — all later phases.
- ❌ Multi-token wallet-balance UI — V1 webapp polish, not pre-Phase-1.
- ❌ Squads 2-of-3 multisig upgrade authority — mainnet-prep gate, not pre-Phase-1.
- ❌ SDK `subscribe()` auto-resub-on-disconnect (Drift v2 pattern) — V1 SDK hooks plan owns this.
- ❌ `@bracketchain/sdk/react` subpath publishing — V1 SDK hooks plan owns this.
- ❌ VRF seeding, on-chain 3rd-Nth placement attestation, `games` table — Phase 1 program work.

---

## Execution log (2026-05-18)

> **⚠️ NOT-SHIPPED BANNER (added 2026-05-19).** The work this log records did not land in any consumed code path. Disk verification on 2026-05-19 found: `bracket-chain-programs/codama.json` absent; `bracket-chain-sdk/src/generated/` absent; SDK's `src/*` still imports from `@coral-xyz/anchor`; SDK's vendored `src/idl/bracket_chain.json` still present. The indexer has an orphaned `src/generated/src/generated/` Codama tree, but no application code imports it (all `../generated/...` imports resolve to `../generated/prisma`). **Treat this log as planning notes from an exploratory session, not as a record of shipped state.** The "Findings the plan didn't anticipate" subsection (numbered 1–9) is still useful — those gotchas remain real and will hit a future implementation attempt — and is worth re-reading before Phase 0 begins.

What was attempted this session, what the plan didn't anticipate, and what was still pending at the time of writing. Preserved as planning addenda — the plan body above is intentionally unchanged so future-you can compare "planned vs actual."

### Stage 1 — Codegen pipeline (Section 2 of plan) — ✅ shipped

Files written/modified across 3 repos (all uncommitted in working tree at time of this entry):

- `bracket-chain-programs/`: `codama.json` (new), `Makefile` (codama-generate + sync-idl-events targets), `package.json` (3 dev-deps), `package-lock.json`, `README.md`.
- `bracket-chain-sdk/`: `src/generated/**` (~30 files: accounts, errors, instructions, pdas, programs, types + index), `README.md`.
- `bracket-chain-indexer/`: `src/generated/**` (Codama tree, kept but excluded from build), `tsconfig.build.json` (added `src/generated/src/**` to exclude), `README.md`.

### Stage 2 — Indexer parser migration — ✅ shipped (Option A locked)

- `bracket-chain-indexer/src/webhooks/event-types.ts` (NEW) — hand-typed interfaces for all 7 Anchor events matching what `BorshCoder` emits (PublicKey, BN, primitives).
- `bracket-chain-indexer/src/webhooks/helius-parser.service.ts` — 7 handler signatures retyped from `Record<string, unknown>` to specific event interfaces; dispatch loop narrows via per-case cast.
- Typecheck passes; runtime smoke test (Task #7) deferred — `scripts/test-parser.mjs` has stale hardcoded payload (pre-Phase-2.5), pre-existing bug logged as followup Task #20.

### Stage 3 — SDK Kit migration — 🟡 in progress (5 of 12 files migrated)

- ✅ `src/client.ts` — Kit-native (`rpc: Rpc<SolanaRpcApi>` + `rpcSubscriptions?` + `signer?: TransactionSigner` + `programAddress: Address`).
- ✅ `src/types.ts` — re-exports Codama account types + `WithAddress` wrappers + deprecated `payoutPreset()` shim for transition.
- ✅ `src/methods/createTournament.ts` — full mutation template (tx composition, ATA conditional, error mapping preserved).
- ✅ `src/methods/joinTournament.ts` — idempotent ATA create + balance pre-check + post-tx `fetchParticipant` (replaces event-log parsing).
- ✅ `src/methods/queries.ts` — `getTournament` + `getProtocolConfig` + composite `getTournamentState`; listings stubbed with informative throws (deferred — production uses `BracketChainIndexerClient` REST).
- ❌ `src/methods/startTournament.ts` — chunked + `@solana-program/compute-budget` for `setComputeUnitLimit` (pending).
- ❌ `src/methods/reportResult.ts` — `remaining_accounts` for final-match placement payouts (pending).
- ❌ `src/methods/cancelTournament.ts` — chunked refunds + remaining_accounts (combines patterns).
- ❌ `src/methods/subscribe.ts` — Task #12 separately (different paradigm — Kit RPC subscriptions, not `connection.onAccountChange`).
- ⏳ `src/pdas.ts`, `src/index.ts` — re-export cleanup pending.
- ⏳ `src/errors.ts` Task #13 — `mapError`'s `AnchorError`-checking branch is now dead code; need Kit `SolanaError` mapping.
- ⏳ `scripts/init-protocol.ts`, `scripts/e2e-demo.ts` — Task #14, Anchor-based.

### Findings the plan didn't anticipate

These are the loadbearing surprises. Each is a "you would have hit this within a day" gotcha that the plan-as-written didn't flag.

1. **Codama-generated clients are `@solana/kit`-style, NOT `@coral-xyz/anchor`-style.** Per the Anchor TS client docs: *"`@anchor-lang/core` is only compatible with v1 of `@solana/web3.js`. It is not compatible with v2 (which Kit re-exports)."* This is the single biggest plan delta — adopting Codama isn't an import swap, it's a paradigm migration. Driven Stage 3's 4-stage scope rewrite.

2. **Codama needs WSL + Node 20 LTS on Windows hosts.** The Codama CLI uses Node ESM dynamic imports with absolute paths, which Node 24's stricter ESM loader rejects on Windows (`Received protocol 'd:'`). Workaround: invoke from WSL with `nvm use 20.20.2` then `node node_modules/@codama/cli/bin/cli.cjs run --all`. The plan's `npx codama run` instructions don't work from native Windows shells.

3. **`@codama/renderers-js@2.x` emits a package wrapper, not flat output.** The Solana docs example shows `clients/js/src/generated/{accounts,instructions,...}` — that's v1 output. v2 wraps in `<dir>/package.json` + `<dir>/src/generated/{accounts,...}`. The generated `package.json` declares peer dep `@solana/kit@^6.4.0` and dep `@solana/program-client-core@^6.4.0` — useful for verifying versions, less useful for direct imports (4-level nesting from SDK src).

4. **`@codama/renderers-js@2.x` does NOT emit event log decoders.** It emits `accounts/`, `errors/`, `instructions/`, `pdas/`, `programs/`, `types/` — no `events/`. Codama IDL spec has `EventNode` (PR #985 in the upstream repo), but the renderer hasn't picked it up. Verified by inspecting `node_modules/@codama/renderers-js` source + checking npm dist-tags (`latest = 2.2.0`, no commits to event support in the last ~3 months of the renderers-js GitHub history). Drives Stage 2's Option A: BorshCoder + EventParser stay for events; sync-idl-events target keeps the events-only IDL JSON fresh.

5. **The package-manager-per-repo mismatch matters.** `bracket-chain-programs` has both `package-lock.json` + `yarn.lock` (Anchor scaffold left both). `bracket-chain-sdk` uses pnpm (`pnpm-lock.yaml`). `bracket-chain-indexer` uses pnpm. `BracketChain-Frontend` uses npm. Running `npm install` in the SDK repo triggered an `npm@11.6.1` arborist bug (`Cannot read properties of null (reading 'matches')`) because it ignored `pnpm-lock.yaml` and rebuilt the dep tree from scratch — exposing transient npm bugs. Fix: use `pnpm add` in the SDK + indexer; npm is OK only in `bracket-chain-programs` (the codama install) and frontend.

6. **`@solana-program/associated-token-account` is NOT a separate package.** ATA helpers (`findAssociatedTokenPda`, `getCreateAssociatedTokenIdempotentInstruction`, `TOKEN_PROGRAM_ADDRESS`) ship inside `@solana-program/token`. Only 6 `@solana-program/*` packages exist on npm — token, system, stake, token-2022, compute-budget, memo. No ATA-account package.

7. **Codama PDA seed-arg names don't always match Rust verbatim.** The Rust seed `[b"participant", tournament, wallet]` becomes `findParticipantPda({ tournament, player })` in TS — `wallet` → `player`. The renderer makes ergonomic choices that don't match Rust source. When in doubt, grep the generated PDA file before assuming seed names.

8. **Kit's tx composition has a lifetime-narrowing quirk inside `pipe()`.** After `setTransactionMessageLifetimeUsingBlockhash`, the message has blockhash lifetime, but TS doesn't propagate the narrowing through `pipe`, so `sendAndConfirmTransactionFactory` (which wants `TransactionWithLastValidBlockHeight`) rejects the signed tx. Workaround: cast at the send site `signedTx as Parameters<typeof sendAndConfirm>[0]`. Upstream Kit may add a narrower signing helper later.

9. **Section 1 (solana-keychain) has nothing to migrate today.** Grep across `bracket-chain-indexer/src/` found zero `KEYPAIR` / `secretKey` / `Keypair.from` references. The MVP indexer's reconciliation cron is purely read-only — no signing path exists yet. Section 1's "migrate the existing keypair" smoke test isn't applicable. Moving Section 1 to "do during Phase 1 cron build" (when the first signing path actually appears — V1's auto-claim cron, VRF-reveal cron, etc.).

### Updated effort estimate

Original plan: ~1.5 weeks solo / ~1 week team-of-2. Reality so far: ~1 session for Stages 1+2+5/12-of-Stage-3, with Stage 3 remainder + Stage 4 still ahead. Revised estimate **3-5 weeks total Phase 0 (solo)** — closer to V1-prep than to a quick prerequisite. The 4-stage breakdown is the truer shape than the 2-section plan.

### Outstanding tasks (cross-references to task tracker IDs)

- **Task #11** (Stage 3 method-file rewrites) — 2/5 mutations done. Remaining: startTournament, reportResult, cancelTournament.
- **Task #12** (subscribe.ts to Kit RPC subscriptions).
- **Task #13** (errors.ts — Kit `SolanaError` mapping; current `AnchorError` branch is dead code).
- **Task #14** (scripts/init-protocol.ts + scripts/e2e-demo.ts + version bump to 0.4.0).
- **Task #15-#17** (Stage 4: frontend boundary via `@solana/web3-compat`).
- **Task #20** (followup: fix `bracket-chain-indexer/scripts/test-parser.mjs` — stale pre-Phase-2.5 hardcoded log payload).

---

## Execution log (2026-05-19)

> **⚠️ NOT-SHIPPED BANNER (added 2026-05-19).** Same caveat as the 2026-05-18 log above — none of the Stage 3 file rewrites recorded below are present in the working tree. SDK's `src/methods/{startTournament,reportResult,cancelTournament,subscribe}.ts`, `src/errors.ts`, and `src/index.ts` all still import from `@coral-xyz/anchor`. The cause-chain `extractCustomErrorCode` walker, the Kit RPC `accountNotifications` subscription, the `AccountRole.WRITABLE` wedges — none of it landed. **Treat this log as planning notes.** The "Findings this session" subsection (numbered 1–5) is still useful — the gotchas about Codama-async builders, optional-account sentinel encoding, the missing `findMatchPda` helper, Kit's compute-unit estimation, and `AccountRole` semantics will all hit a future implementation attempt and are worth re-reading.

Continuation session — recorded plans for Tasks #11 / #12 / #13 / partial #14 (Stage 3 SDK rewrites). At time of writing, this log claimed Stage 3 effectively complete from the SDK side; subsequent disk verification (also dated 2026-05-19) found the work absent.

### Stage 3 — SDK Kit migration — ✅ effectively complete

All five files the prior session deferred are now Kit-native:

- ✅ `src/methods/startTournament.ts` — chunked tx + `getSetComputeUnitLimitInstruction` from `@solana-program/compute-budget` + per-chunk fresh blockhash + hand-rolled `findMatchPdaKit` (Codama renderers-js@2.x does not emit a PDA helper for `[b"match", tournament, u8 round, u16_le match_index]` seed shape) + `AccountRole.WRITABLE` wedge for match PDAs as remaining accounts.
- ✅ `src/methods/reportResult.ts` — `getReportResultInstructionAsync` auto-resolves protocolConfig + vault; non-final and final branches share `sendSingle` helper; final-match `nextMatch` uses program-address sentinel for Codama's optional-account "None" convention; idempotent ATA pre-create for placements + treasury.
- ✅ `src/methods/cancelTournament.ts` — first-chunk-only organizer ATA + idempotent ATA-create pre-instruction; per-chunk fresh blockhash; `[participantPda, ATA]` pairs wedged into remaining accounts as `AccountRole.WRITABLE`. Requires explicit `participantWallets` (auto-discover via raw `getProgramAccounts` deferred to the same Stage 3 cleanup as queries.ts listings).
- ✅ `src/methods/subscribe.ts` — Kit's `rpcSubscriptions.accountNotifications(addr, { encoding: 'base64' }).subscribe({ abortSignal })` async-iterator pattern; one `AbortController` drives teardown of every subscription registered by a single call; base64 → bytes via `getBase64Encoder().encode(s)`; clean shutdown via `signal.aborted` check distinguished from connection-level errors.
- ✅ `src/errors.ts` — `mapError`'s dead `AnchorError` branch replaced with cause-chain walker (`extractCustomErrorCode`) that pattern-matches Kit's `SolanaError<SOLANA_ERROR__INSTRUCTION_ERROR__CUSTOM>` across up to 5 cause hops. The 21 typed `BracketChain*` error subclasses + `ERRORS_RS_ORDER` + `ON_CHAIN_TO_SDK` map preserved verbatim — only dispatch logic changed. Unmapped on-chain codes (still in our 6000–6023 range) fall through to `TransactionFailedError` with Codama's `getBracketChainErrorMessage(code)`.

### Index.ts cleanup — ✅ shipped

Removed stale type re-exports that the new Codama-backed types module no longer carries: `TournamentStatusKind`, `MatchStatusKind`, `PayoutPresetVariant`, `TournamentStatusVariant`, `MatchStatusVariant`, `PublicKey` re-export, `getEnumKind` helper. Added `MatchStatus`, `PayoutPreset`, `TournamentStatus` numeric-enum re-exports.

### Typecheck status

Main `pnpm typecheck` clean except for one error:

```
src/methods/startTournament.ts(41,51): error TS2307: Cannot find module '@solana-program/compute-budget' or its corresponding type declarations.
```

Resolution = user install: `pnpm add @solana-program/compute-budget@^0.10.0`. Verify version matches release train of `@solana-program/system@^0.12.0` + `@solana-program/token@^0.13.0` already in lockfile.

### Scripts (Task #14 — partial)

- `scripts/init-protocol.ts` — still works against the legacy Anchor + legacy IDL path (imports `findProtocolConfigPda` from src which resolves to the un-migrated `pdas.ts`). No action this session.
- `scripts/e2e-demo.ts` — 589 LOC integration test with web3.js Keypair / Connection / SystemProgram / spl-token mint helpers throughout. Original 30-min estimate in Step 6 was off — full migration is ~2-3h. Deferred. Added deferred-migration header at top of file with concrete migration scope notes. **Important**: scripts have a separate `scripts/tsconfig.json` and are NOT in the main `pnpm typecheck` or `tsup` build pipeline, so e2e-demo's broken state does NOT block publishing 0.4.0.

### Findings this session

1. **Codama's `getReportResultInstructionAsync` and `getCancelTournamentInstructionAsync`** auto-resolve protocolConfig, vault, and tokenProgram PDAs via the generated PDA helpers, eliminating boilerplate that lived in the legacy SDK. Confirmed by reading the generated source — they call `findProtocolConfigPda()` / `findVaultPda()` internally when the input field is omitted.

2. **Codama renderers-js@2.x optional-account encoding sentinel = program address.** For instructions with optional accounts (`reportResult.nextMatch` for final-match path, `cancelTournament.organizerTokenAccount` after first chunk), "None" is expressed by passing the program's own address. Confirmed in `parseReportResultInstruction` / `parseCancelTournamentInstruction`'s `getNextOptionalAccount` reader (`accountMeta.address === BRACKET_CHAIN_PROGRAM_ADDRESS ? undefined : accountMeta`).

3. **`findMatchPda` is NOT auto-generated by Codama** because the seed shape `[b"match", tournament, [u8 round], u16_le match_index]` mixes byte literal + Address + raw u8 + raw u16 LE — Codama's renderer only emits PDA helpers for seed shapes that fit its byte/utf8/address encoders. Hand-rolled `findMatchPdaKit` using `getProgramDerivedAddress` lives inline in both startTournament.ts and reportResult.ts (duplicated until pdas.ts itself is Kit-migrated).

4. **Kit ships compute-unit estimation natively** as of Kit changelog #1476 — `estimateComputeUnitLimitFactory` + `estimateAndSetComputeUnitLimitFactory` moved from `@solana-program/compute-budget` into `@solana/kit` itself. The instruction-builder helper (`getSetComputeUnitLimitInstruction`) still lives in `@solana-program/compute-budget` — that's why we still need the package. Future polish: replace our fixed `400_000` per chunk with `estimateAndSetComputeUnitLimitFactory(rpc)`.

5. **`AccountRole.WRITABLE = 1`** is the canonical Kit enum value for non-signer + writable accounts in instruction metas (`READONLY=0, WRITABLE=1, READONLY_SIGNER=2, WRITABLE_SIGNER=3`; bit0 = signer, bit1 = writable). Confirmed in `kit/packages/instructions/src/roles.ts`.

### Outstanding tasks (updated)

- **Task #11** — ✅ Done (all 5 mutations migrated).
- **Task #12** — ✅ Done (subscribe.ts Kit RPC subscriptions).
- **Task #13** — ✅ Done (errors.ts SolanaError dispatch).
- **Task #14** — 🟡 Partial. `init-protocol.ts` works as-is on legacy path. `e2e-demo.ts` deferred with migration-scope header. Version bump 0.3.1 → 0.4.0 pending user action.
- **Task #15-#17** (Stage 4 frontend boundary) — ⏳ Not started.
- **Task #18** (this roadmap + plan update) — ✅ Done 2026-05-19.
- **Task #20** (test-parser.mjs hardcoded payload) — ⏳ Not addressed this session.

### Updated effort estimate

Original plan: ~1.5w solo. Mid-session revision (2026-05-18): ~3-5w solo (4-stage scope discovered). Actual so far: **2 working sessions** for Stages 1+2+3, plus Stage 4 still ahead (estimated ~1-2 days for the frontend bridge alone — separate `BracketChain-Frontend` changes). Total realistic Phase 0 end-to-end: **2-3 weeks elapsed time solo**, much faster than the 3-5w worst-case projection because the Async-variant Codama builders eliminated more boilerplate than expected.

---

## Next steps (resume-from-here)

Concrete pickup plan when this session ends and the next one starts. Ordered by what compounds best — earlier items unblock later ones.

### Step 1 — Commit the working tree (~10 min)

The 4-repo working tree has accumulated significant uncommitted change across Stages 1+2+partial-3. Cost of staying uncommitted longer: a system crash, IDE corruption, or accidental `git restore` loses several hours of work that's only regenerable via re-execution.

Recommended: 3 commits, per repo (the polyrepo tax is per-repo). The frontend has no changes in this session and stays untouched.

```
bracket-chain-programs:  "phase 0 stage 1: adopt Codama codegen pipeline"
bracket-chain-sdk:       "phase 0 stage 3 (partial): Kit-native client + 2 mutations + queries"
bracket-chain-indexer:   "phase 0 stage 2: typed event interfaces + parser migration (Option A)"
```

If the SDK's mid-state ("doesn't typecheck") feels uncomfortable to commit, the alternative is to delay the SDK commit until Stage 3 finishes — but then the SDK changes accumulate further before any checkpoint.

### Step 2 — Update `bracketchain-roadmap.md` (~20 min)

Second half of Task #18. The roadmap currently says Phase 0 is "1.5 weeks solo / 1 week team-of-2" and lists R1 as "Codama not yet implemented." Update:
- Phase 0 row: "In progress (4-stage scope discovered); 3-5 weeks revised estimate solo"
- Risk register R1: "Codama landed for accounts/types/instructions; partial — events still on BorshCoder per Option A. SDK Kit migration in progress."
- Action items section: check off "Phase 0 — Codama pipeline setup + CI gate" with notes ("CI gate deferred per Stage 1 decision; local regen discipline applies").

### Step 3 — Finish Task #11 (Stage 3 mutations, ~2-3 hours)

Three remaining method files, ordered by complexity:

1. **`startTournament.ts`** (~45 min) — chunked txs + `@solana-program/compute-budget` for `setComputeUnitLimit(1_400_000)` on each chunk. Pattern: loop over chunk indices, build per-chunk tx with `[computeBudgetIx, startIx]`, sendAndConfirm sequentially. The seed-derivation client-side (Fisher-Yates from `tournament.seedHash`) stays on the SDK side.

2. **`reportResult.ts`** (~45 min) — verify how `getReportResultInstructionAsync` exposes `remaining_accounts` for the final-match branch (placement payouts). If the generated builder doesn't natively accept variadic accounts, modify the returned `Instruction.accounts` array directly (the type allows `TRemainingAccounts extends readonly AccountMeta<string>[] = []`).

3. **`cancelTournament.ts`** (~45 min) — combines startTournament's chunking with reportResult's remaining_accounts (chunks of participant ATAs to refund). The current SDK has a tx-size-budget helper at the top of the file that should port mostly unchanged (it's just arithmetic on AccountMeta sizes).

Each typechecks in isolation after writing. End state: all 5 mutations + queries + types + client compile clean. Subscribe + scripts + errors still pending.

### Step 4 — Task #12 (subscribe.ts, ~1 hour)

Kit's RPC subscriptions are a different API surface than `connection.onAccountChange`:

```ts
const subscription = await client.rpcSubscriptions
  .accountNotifications(pda, { commitment: 'confirmed', encoding: 'base64' })
  .subscribe({ abortSignal });

for await (const notification of subscription) {
  const account = decodeTournament({...});  // or decodeMatchNode
  callback({ kind: 'tournament', tournament: account.data });
}
```

Async-iterator-based, AbortSignal-driven. The existing `subscribe()` API surface (`onError`, `kind: 'tournament'|'match'` discriminator) should stay so frontend's `useTournamentView` doesn't need to change.

Defer auto-resub-on-disconnect — that's an explicit V1 SDK hooks plan item (`bracketchain-v1-sdk-hooks-plan.md`).

### Step 5 — Task #13 (errors.ts Kit mapping, ~1 hour)

Replace the `mapError`'s `AnchorError` branch with Kit's `SolanaError` pattern matching. Codama generates a `getBracketChainErrorMessage(code)` function in `src/generated/src/generated/errors/bracketChain.ts` — wire it into the existing class-based mapping table. The 21 typed BracketChain* error classes themselves don't need to change; only the lookup logic does.

### Step 6 — Task #14 (scripts + version bump, ~30 min)

`scripts/init-protocol.ts` and `scripts/e2e-demo.ts` are CLI scripts that import the SDK's public API. After Stage 3 finishes they need updating to call `createTournament(client, {...})` etc. with the new bigint+enum shapes. Bump SDK `package.json` version to `0.4.0` (paradigm-break minor bump pre-1.0).

### Step 7 — Stage 4 (Tasks #15-#17, ~1-2 days)

Frontend boundary work — separate from SDK Stage 3.

1. **Task #15**: `pnpm add @solana/web3-compat` in `BracketChain-Frontend/`.
2. **Task #16**: In `BracketChain-Frontend/lib/sdk.ts`, bridge `useAnchorWallet()` output to Kit's `TransactionSigner` shape via `@solana/web3-compat`'s `convertWalletAdapterToKit` helper (or equivalent — verify on install). Centralize so all hooks consume one bridge.
3. **Task #17**: `pnpm dev` + browser smoke: connect Phantom → `/create` → submit → switch wallets → `/join` → `/t/[id]` report match. Verify Sonner toasts fire correctly. `make check` (lint + type-check) must pass.

Also: SDK consumers (frontend) currently pull `@bracketchain/sdk@0.3.1` from npm. After Stage 3 + version bump to 0.4.0, either:
- **Option A (cleanest)**: publish 0.4.0 to npm, frontend bumps its dep.
- **Option B (during transition)**: use `pnpm link` to point frontend at the local SDK working tree. Faster iteration but spooky if the link gets out of sync.

### Step 8 — Phase 0 closeout

Re-run all top-level acceptance gates above. If all green except keychain (Section 1 — deferred to Phase 1 cron build), call Phase 0 done. Update `bracketchain-roadmap.md` risk register R1+R3 status. Begin Phase 1 V1.1 work.

---

### Where to STOP Phase 0 and start Phase 1 instead

If at any point during Step 3-7 it becomes clear the SDK rewrite is taking longer than expected, **don't sunk-cost it**. Phase 1's program-side work (V1.1 SAS identity + V1 player-reported settlement + V1.2 oracle + program-improvements + partial-cancel — all bundled in one redeploy per the roadmap) is Rust code; it doesn't depend on the SDK migration being complete.

Reasonable cutover points:
- **After Step 3 + 4** (Stage 3 mutations + subscribe): SDK compiles end-to-end. Frontend still on @bracketchain/sdk@0.3.1 from npm (legacy Anchor pathway). Phase 1 program work can proceed; SDK gets re-published as 0.4.0 alongside the V1.1 redeploy anyway.
- **After Step 3 only** (mutations done, subscribe pending): same story, with the subscribe rewrite as Phase 1 hygiene rather than Phase 0 prerequisite.

The principle: Phase 0's purpose is "Codama pipeline + clean keypair pattern in place for Phase 1 cron services." The pipeline is in place. The SDK Kit migration is *nice-to-have* for Phase 1, not load-bearing. If Stage 3+4 stretches past 2 more weeks of solo time, hand off to Phase 1 and finish Stage 3+4 as you go.

---

## Update protocol

When Phase 0 acceptance gates pass:
1. Mark all gates ✅ in this file.
2. Update `bracketchain-roadmap.md` risk register: R1 → ✅ Resolved; R3 → ✅ Resolved.
3. Update `bracketchain-roadmap.md` action items: Phase 0 work checked off.
4. Begin Phase 1 program work per V1.1 plan.
5. This file becomes historical reference; safe to delete after Phase 1 ships (or keep as Phase 0 retrospective).
