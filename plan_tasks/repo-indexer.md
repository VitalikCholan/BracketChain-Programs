# BracketChain-Indexer — план робіт

> NestJS 11 + Prisma 7 + Postgres (Neon, Railway). Path: `/home/min/git-workspace/BracketChain-Indexer/`.
> Поточний стан: 4 Prisma models (Tournament/Participant/Match/Payout), 1 SQL view `protocol_fees`, Helius webhook (unauthenticated), 60s reconciliation cron, REST endpoints. Production: `https://bracketchain-indexer-production.up.railway.app`.

---

## Phase 0 — pre-Phase-1 hygiene

### Section 1 — `solana-keychain` (DEFERRED to Phase 1)

> Свідомо відкладено 2026-05-19: MVP indexer не має signing path для міграції. Section 1 lands коли перший signing cron буде built (V1 auto-claim / VRF-reveal / partial-refund / close-terminal).

### Section 2 — Codama parser migration (Stage 2)

- [ ] **Крок 24.** Stage 2.A — `src/webhooks/event-types.ts` з hand-typed interfaces для 7 events. Matches `BorshCoder` output shapes. (Codama renderers-js@2.x не emit event decoders → Option A: BorshCoder для events + Codama для accounts/instructions.)
- [ ] **Крок 25.** Stage 2.B — переписати `src/webhooks/helius-parser.service.ts`: 7 handlers з `Record<string, unknown>` → specific event interfaces. Dispatch через per-case type narrowing. Acceptance: `pnpm typecheck` clean.
- [ ] **Крок 26.** Stage 2.C — fix `scripts/test-parser.mjs` (Task #20): захопити current post-Phase-2.5/2.6 webhook payload з devnet, замінити stale hardcoded payload.

### Section 3.1 — Webhook HMAC authentication ⚠️ Security gap

- [ ] **Крок 7.** `pnpm install` + `npm install @types/crypto-js` (якщо потрібно); перевірити NestJS raw-body middleware.
- [ ] **Крок 8.** `src/webhooks/helius-hmac.guard.ts` — NestJS `CanActivate`. Логіка: `X-Helius-Signature` → HMAC-SHA256(raw_body, secret) → `crypto.timingSafeEqual`.
- [ ] **Крок 9.** `@UseGuards(HeliusHmacGuard)` в `webhooks.controller.ts` + `bodyParser.raw({ type: 'application/json' })` в `main.ts`.
- [ ] **Крок 10.** Uncomment `HELIUS_WEBHOOK_SECRET=""` в `.env.example`; generate `openssl rand -hex 32`; Railway env var; Helius dashboard webhook config. Acceptance: `curl POST` без signature → 401; з valid HMAC → 200.

### Section 3.2 — Test coverage

- [ ] **Крок 27.** `src/webhooks/helius-parser.service.spec.ts` — 1 happy-path + 1 re-delivery test на кожне з 7 events. Mock Prisma через `jest-mock-extended` `DeepMockProxy<PrismaClient>`.
- [ ] **Крок 28.** `src/reconciliation/reconciliation.service.spec.ts` — mock `ChainReaderService.fetchTournament` повертає status/champion/slot drift cases. Acceptance: `pnpm test` зелений; runtime <10s.

### Section 3.3 — Name-check endpoint

- [ ] **Крок 11.** `src/tournaments/dto/check-name.dto.ts` — валідація organizer + name query params.
- [ ] **Крок 12.** `GET /tournaments/check-name?organizer=&name=` в `tournaments.controller.ts`. Handler через `prisma.tournament.findUnique({ where: { organizer_name: { organizer, name } } })`. Response: `{ taken: boolean, address?: string }`.
- [ ] **Крок 13.** Smoke-test локально + проти Railway. Free name → `{ taken: false }`; taken → `{ taken: true, address: "..." }`.

---

## Phase 1 pre-flight

- [ ] **Крок 40.** Згенерувати 5 funded keypairs на devnet: `sas-issuer`, `claim-payer`, `vrf-payer`, `refund-payer`, `cleanup-payer`. Airdrop + base58 secrets через `solana-keychain` MemoryBackend env vars.
- [ ] **Крок 41.** SAS bootstrap: `scripts/sas-bootstrap.ts` — створює Credential PDA + Schema PDA з `game_identity` shape (`{ game: u8, steam_id_64: u64, identity_bytes: [u8; 32] }`).
- [ ] **Крок 42.** Switchboard On-Demand devnet smoke — створити test PullFeed з sample OracleJob, request randomness, verify on-chain.

---

## Phase 1 — Indexer work

### Prisma schema migrations (Phase 1 redeploy)

- [ ] **Крок 92.** Prisma migration:
  - `Participant.identity_hash`
  - `Match.proposal_source`, `proposer`, `proposed_winner`, `proposed_at`, `claim_deadline`, `disputed`, `dispute_reason`
  - Новий `Notification` table
  - `MatchCommitment` columns (lobby_id, player_a/b_game_id, committed_at)
  - `TournamentStatus.PartialCancelled` enum variant
  - `prisma migrate deploy` на Railway.

### Event parsing additions

- [ ] **Крок 66.** Парсити 4 нові events:
  - `ResultProposed`
  - `ResultDisputed`
  - `ResultClaimed`
  - `DisputeResolved`
- [ ] `event_version: u8` reject на mismatch → `unknownEventVersion` metric.
- [ ] V1.2 events (`MatchLobbyCommitted`, etc.).
- [ ] V1 partial-cancel event (`TournamentPartiallyCancelled`).

### Cron services (4 нові)

- [ ] **Крок 67.** `src/crons/auto-claim.cron.ts` — scan matches з `proposal_source != None && !disputed && now >= claim_deadline`, call `claim_result` (signed by `claim-payer`). Emit Notification.
- [ ] **Крок 68.** `src/crons/vrf-reveal.cron.ts` — scan tournaments з `vrf_randomness_account != null && !seed_revealed && now > commit_slot + N`, call `reveal_seed` (signed by `vrf-payer`).
- [ ] **Крок 79.** `src/crons/oracle-relayer.cron.ts` — scan matches з bound feed, last feed update > N slots, no proposal yet, call `propose_result_oracle` (signed by `claim-payer`).
- [ ] **Крок 87.** `src/crons/partial-refund.cron.ts` — drives `partial_refund_chunk` chunks до завершення (signed by `refund-payer`).
- [ ] **Крок 83.** `src/crons/close-terminal.cron.ts` — scan completed tournaments > 7 днів, call `close_tournament` chunks (signed by `cleanup-payer`).
- [ ] **R2 mitigation.** Будувати `PermissionlessDriver` cron abstraction коли 4 нові cron'и land — НЕ pre-Phase-1.

### Identity attestation (V1.1)

- [ ] **Крок 52.** `src/identity/identity.controller.ts` з `POST /identity/steam/attest`. Використовує `solana-keychain` для `sas-issuer` role. Issue SAS attestation via CPI.

### Notifications (V1 player-reported narrow kernel)

- [ ] Notification table + WebSocket subscription endpoint `/notifications/subscribe`.
- [ ] Push delivery hooks для `ResultProposed`/`ResultClaimed`/`ResultDisputed`/`DisputeResolved`.

---

## Phase 2 — Onboarding (no program changes)

### Webapp Phase A — Dashboard + Discovery

- [ ] `GET /organizers/:wallet/analytics` — aggregates (tournaments_count, total_pool, total_players).
- [ ] Розширити `GET /tournaments` фільтрами (prize/entry/status/sort).

### Webapp Phase B — Notifications (повний event bus)

- [ ] Розширити narrow kernel з Phase 1: `matchReady`, `payoutReceived`, `tournamentStarting`, `tournamentCancelled`, dispute events.
- [ ] Per-wallet notification preferences.
- [ ] Web Push registration endpoint.

---

## Phase 3 — Engagement

- [ ] `player_stats` view (aggregates з Participant + Match).
- [ ] cNFT badge eligibility cron + minter cron (Metaplex Bubblegum, Helius DAS read).
- [ ] V2-A Sponsor: `Sponsorship` table, `SponsorshipReceived` notifications.

---

## Phase 4 — B2B (optional)

- [ ] V2 dist C — Analytics tier endpoints (tournament-level dashboards, retention cohorts, revenue analytics, CSV export). `WalletTier` table.

---

## Critical files (quick reference)

**Phase 0:**
- `src/webhooks/helius-hmac.guard.ts` (new, 3.1)
- `src/webhooks/webhooks.controller.ts` — `@UseGuards` (3.1)
- `src/main.ts` — raw-body middleware (3.1)
- `.env.example` — uncomment `HELIUS_WEBHOOK_SECRET` (3.1)
- `src/webhooks/helius-parser.service.spec.ts` (new, 3.2)
- `src/reconciliation/reconciliation.service.spec.ts` (new, 3.2)
- `scripts/test-parser.mjs` — Task #20 fix (3.2)
- `src/tournaments/{controller,service}.ts` — `check-name` endpoint (3.3)
- `src/generated/` — Codama output (Section 2)

**Phase 1:**
- `src/keys/keychain.module.ts` (new, Section 1 — Phase 1)
- `prisma/schema.prisma` — schema migration
- `src/crons/{auto-claim,vrf-reveal,oracle-relayer,partial-refund,close-terminal}.cron.ts`
- `src/identity/identity.controller.ts` (new, SAS attestation)
- `src/notifications/` (new)
