# Послідовність робіт по репо

> Який репо коли робити. Cross-reference: [`repo-programs.md`](./repo-programs.md), [`repo-indexer.md`](./repo-indexer.md), [`repo-sdk.md`](./repo-sdk.md), [`repo-frontend.md`](./repo-frontend.md).

---

## Phase 0 (~4-6 тижнів solo)

### Крок A — паралельно (1-2 тижні)

Два потоки **незалежні**, можна робити одночасно:

**Потік 1 — Indexer (3-5 днів):**
1. `Indexer` Section 3.1 HMAC guard
2. `Indexer` Section 3.2 tests (parser + reconciliation)
3. `Indexer` Section 3.3 `check-name` endpoint
4. → `Frontend` `useNameCheck` hook (залежить від 3.3)

**Потік 2 — Programs (1-2 дні):**
1. `Programs` Section 3.4 — `organizer-deposit.test.ts` (3 тести)
2. `Programs` Section 3.4 — `capacity-128p-deep.test.ts` + `CU_BUDGET.md`

### Крок B — Codama (sequential, ~3-5 тижнів)

⚠️ **Тут чітка послідовність** — кожен крок розблоковує наступний:

```
Programs (Stage 1)
   ↓ codama.json + npx codama run --all
   ↓ src/generated/ закомічено в Programs+SDK+Indexer
   ↓
   ├── Indexer (Stage 2) ──► parser migration на typed events
   │                          (паралельно з SDK Stage 3)
   │
   └── SDK (Stage 3) ──► Anchor → Kit rewrite (7 sub-stages)
            ↓ pnpm publish @bracketchain/sdk@0.4.0
            ↓
            Frontend (Stage 4) ──► web3-compat bridge
                                    smoke-test create→join→report→cancel
```

**Чому саме так:**
- SDK не може почати Stage 3 без `src/generated/` від Programs Stage 1.
- Frontend не може мігрувати без SDK 0.4.0 на npm.
- Indexer Stage 2 **не блокує** SDK — паралельно.

---

## Phase 1 (~12-15 тижнів solo, single redeploy)

### Крок C — Programs first (8-10 тижнів)

Все це **в одному redeploy** — порядок усередині repo важливий, але outside repo нічого не починаємо:

```
Programs:
1. V1.1   game.rs + SAS extensions + event_version: u8
2. V1     proposal envelope + VRF + 9 dispute ix
3. V1.2   oracle commitment + 3 oracle ix
4. Improv PayoutPreset::Custom + close_tournament
5. PCanc  partial_cancel + partial_refund_chunk
   ↓
6. anchor build + tests green
7. anchor deploy --provider.cluster devnet → новий program ID
```

### Крок D — propagation (1 тиждень)

Після deploy strictly sequential:

1. **`Programs`** → `make codama-generate` (regenerates SDK + Indexer clients)
2. **`SDK`** → bump 0.4.0 → 0.5.0, додати ~15 method files, `pnpm publish`
3. **`Indexer`** → Prisma migration (`prisma migrate deploy`), bootstrap (5 keypairs через keychain, set_sas_config, set_oracle_config)
4. **`Indexer`** → 5 нових cron services
5. **`Indexer`** → SAS identity controller

### Крок E — Frontend last (4-5 тижнів)

```
Frontend:
1. Steam OpenID flow (V1.1)
2. ReportResultModal rewrite (V1)
3. BindFeedModal (V1.2)
4. Update SDK dep → 0.5.0
```

---

## TL;DR — порядок репо

| # | Phase | Репо | Що |
|---|---|---|---|
| 1 | 0 | **Indexer** + **Programs** | паралельно: Section 3 gaps |
| 2 | 0 | **Programs** | Codama Stage 1 (codegen) |
| 3 | 0 | **SDK** + **Indexer** | паралельно: Stage 2/3 (parser + Kit rewrite) |
| 4 | 0 | **Frontend** | Stage 4 (web3-compat bridge) |
| 5 | 1 | **Programs** | full bundle → redeploy |
| 6 | 1 | **SDK** | bump 0.5.0 |
| 7 | 1 | **Indexer** | Prisma + crons + identity |
| 8 | 1 | **Frontend** | Steam OpenID + Report/BindFeed UI |

**Bottleneck:** Programs — без redeploy ніщо в Phase 1 не рухається. Codama Stage 1 в Programs — без нього Stage 3 SDK заблокований.

**Найкритичніший крок:** Step 49 (V1.1 SAS attestation validation в `join_tournament.rs`) — 1 повний день з ручним `validate_attestation()` helper'ом. Anchor `#[account(constraint = ...)]` цього не handles.

---

## Phase 2-3 (post-Phase-1, без program changes)

Після того як Phase 1 redeploy в production, repo sequence стає:

| Phase | Репо | Тривалість | Що |
|---|---|---|---|
| 2 | **Indexer** + **SDK** | 2-3 тижні | analytics endpoints + `@bracketchain/sdk/react` subpath (`useTournament`/`useBracket`/`useEscrow` + Drift v2 auto-resub) |
| 2 | **Frontend** | 4-6 тижнів | Privy embedded wallets (B2C unlock) + Dashboard + Notifications full event bus |
| 3 | **Indexer** | 1-2 тижні | `player_stats` view + cNFT eligibility cron + badge minter cron |
| 3 | **Frontend** | 4-6 тижнів | Profile + cNFT badges + Sponsor UI + Mobile UX polish |
| 3 | **Widget package** | 2-3 тижні | новий npm package для Twitch embedding (<80KB gzipped) |
