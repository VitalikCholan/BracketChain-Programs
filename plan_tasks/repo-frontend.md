# bracketchain-frontend — план робіт

> Next.js 16 + React 19, Vercel. Path: `/home/min/git-workspace/bracketchain-frontend/`.
> Поточний стан: 6 routes (`/`, `/about`, `/create`, `/explore`, `/dashboard`, `/t/[id]`). Wallet adapter (Phantom + Solflare via Wallet Standard). SDK 0.3.0 повністю wired (organizer deposit, error mapping → 11 typed branches → toasts + inline errors). SWR pattern (indexer first → RPC fallback). Sonner toasts.

---

## Phase 0 — pre-Phase-1 hygiene

### Section 3.5 — README ✅ DONE

- [x] **Крок 6.** README rewrite — більше нема "simulated transaction" / "pre-SDK" claims (R11 resolved).

### Section 3.3 follow-up — Name-check hook

- [ ] **Крок 14.** `features/tournament/create/hooks/useNameCheck.ts`:
  - debounce 300ms `useEffect` calling `GET /tournaments/check-name?organizer=&name=`
  - Заміна mock в `features/tournament/steps/DuplicateNameWarning.tsx:30` (TODO `replace with real API call`).
- [ ] **Залежить від:** Indexer Section 3.3 endpoint.

### Stage 4 (Codama) — SDK boundary via `@solana/web3-compat`

- [ ] **Крок 36.** Stage 4.A — `pnpm add @solana/web3-compat`.
- [ ] **Крок 37.** Stage 4.B — `lib/sdk.ts` — bridge `useAnchorWallet()` output → Kit `TransactionSigner` через `@solana/web3-compat`. Reuse в усіх SDK-consuming hooks.
- [ ] **Крок 38.** Stage 4.C — `pnpm link` SDK локально для swift iteration. Smoke test: connect Phantom → /create → join → report → cancel. Acceptance: всі toasts fire, no error toasts, indexer SWR works.
- [ ] **Крок 39 (follow-up).** Bump frontend dep → `@bracketchain/sdk@^0.4.0`.

---

## Phase 1 — V1.1 Steam OpenID + SAS attestation

- [ ] **Крок 51.** Steam OpenID flow в `features/auth/steam/`:
  - User clicks "Link Steam" → redirect to Steam OpenID endpoint.
  - Callback повертає Steam ID 64.
  - `POST /identity/steam/attest` → indexer issues SAS attestation CPI.
  - Store attestation pubkey в local user profile state.
  - Подальше `joinTournament` включає attestation account в transaction.
- [ ] **Effort:** ~3-4 дні frontend dev.

---

## Phase 1 — V1 Player-reported UI

- [ ] **Крок 69.** Rewrite `ReportResultModal` як action-dispatcher:
  - Routes між propose / confirm / dispute / claim / resolve panels на основі:
    - **Viewer:** `player_a` | `player_b` | `organizer` | `other`.
    - **Match state:** `Active+None` | `Active+proposed` | `Disputed` | `Past-deadline`.
  - **Effort:** ~1 тиждень frontend dev.

---

## Phase 1 — V1.2 Oracle UI

- [ ] **Крок 78.** `BindFeedModal` + Oracle-pending panel. Organizer creates feed + binds to match PDA в один UX flow.
- [ ] **Effort:** ~3-4 дні frontend dev.

---

## Phase 1.5 — Real Oracle wire-up

- [ ] **Крок 100.** `BindFeedModal` full implementation: organizer creates feed, binds to match PDA в один UX flow (mocked feeds → real Switchboard On-Demand).

---

## Phase 2 — Onboarding (B2C unlock)

### Webapp Phase C — Privy (THE B2C unlock)

- [ ] Інтегрувати Privy embedded wallets:
  - Email / Google login → auto-created Solana embedded wallet.
  - User signs in без жодного wallet install.
  - MoonPay / Coinbase Pay fiat ramp ($10 USD → USDC <5min).
- [ ] Зберегти external-wallet (Phantom) login path; optional email link для notifications.
- [ ] **R8 mitigation:** Privy app config (recovery, security) — resolve before Phase 2 begins.
- [ ] **Acceptance gates:**
  - Email login → embedded wallet → join tournament без Phantom install.
  - MoonPay flow <5min.

### Webapp Phase A — Dashboard + Discovery

- [ ] Organizer dashboard ("my tournaments", analytics aggregates).
- [ ] Tournament browser з фільтрами (prize/entry/status/sort).

### Webapp Phase B — Notifications (full event bus)

- [ ] Service Worker registration.
- [ ] Web Push: `matchReady`, `payoutReceived`, `tournamentStarting`, `tournamentCancelled`, dispute events.
- [ ] Per-wallet notification preferences page.

### SDK Hooks integration

- [ ] Замінити custom `useReducer` hooks на `@bracketchain/sdk/react` (`useTournament`, `useBracket`, `useEscrow`).
- [ ] Pull-to-refresh на tournament page → indexer (fast) → RPC fallback if stale.

---

## Phase 3 — Engagement & Reach

### Webapp Phase D — Profile + cNFT Badges

- [ ] Public `/profile/[wallet]` route.
- [ ] Aggregated stats з `player_stats` view (Indexer).
- [ ] cNFT badges via Helius DAS read API (Metaplex Bubblegum).
- [ ] Tournament history + match-record chart.

### V2-A Sponsor flow

- [ ] Sponsor injection UI: будь-який wallet → inject funds в Active vault з 5% fee.
- [ ] Sponsor leaderboard.
- [ ] `SponsorshipReceived` push notifications.

### Mobile UX polish (explicit Phase 3 deliverable, не в плані)

- [ ] Responsive bracket view (horizontal scrolling, не grid).
- [ ] Touch-first interaction.
- [ ] PWA installable.
- [ ] Performance-budget audits.
- [ ] **Acceptance:** full create-join-play flow на iPhone Safari + Android Chrome.

### Custom payouts UI

- [ ] Розблокувати currently-gated MVP-only preset UI → 4 presets (WTA / Standard / Deep / **Custom**).
- [ ] Custom UI: 8-slot basis-points slider, sum validation (== 10_000).

### Format picker activation

- [ ] Frontend вже має UI для double-elim / Swiss / RR (per `[[project_v1_format_expansion]]`).
- [ ] Activate коли Programs ship formats Phase A/B/C ix.

---

## Phase 4 — B2B (optional)

- [ ] White-label tenant config (custom domain + branding + feature toggles).
- [ ] Analytics tier gating (Pro/Enterprise wallet flag → unlock dashboards).

---

## Critical files (quick reference)

**Phase 0:**
- `BracketChain-Frontend/README.md` ✅ done
- `features/tournament/create/hooks/useNameCheck.ts` (new, 3.3 follow-up)
- `features/tournament/steps/DuplicateNameWarning.tsx` — replace mock
- `lib/sdk.ts` — `web3-compat` bridge (Stage 4.B)

**Phase 1:**
- `features/auth/steam/` (new — V1.1 Steam OpenID)
- `features/tournament/report/ReportResultModal.tsx` — full rewrite (V1 player-reported)
- `features/tournament/oracle/BindFeedModal.tsx` (new — V1.2)

**Phase 2:**
- `features/auth/privy/` (new — Phase C)
- `features/dashboard/` (Phase A)
- `features/notifications/` (Phase B)
- Replace hooks → `@bracketchain/sdk/react`

**Phase 3:**
- `app/profile/[wallet]/page.tsx` (new — Phase D)
- `features/sponsor/` (V2-A)
- Mobile responsive overhaul

---

## Strategic note

Privy Phase C — **це** B2C unlock. Без нього BracketChain = "for crypto users who happen to play Dota". З ним = "for Dota players who happen to want trustless prizes". Order-of-magnitude adoption difference.

Mobile UX polish — explicit Phase 3 deliverable, не в жодному плані (R6 у risk register). Це означає: frontend dev повинен забюджетити окремий час, він не "просто з'явиться" з features.
