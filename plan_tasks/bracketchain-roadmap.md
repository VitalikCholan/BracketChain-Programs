# BracketChain Roadmap

> **Living document.** This is the canonical source of truth for plan sequencing, ownership-locked decisions, and phase composition across the BracketChain protocol. When priorities shift, update this file **first**; per-plan changes follow. If you find yourself editing multiple plan files for a sequencing decision, stop and update this roadmap first.

**Status:** MVP shipped (devnet, 2026-05-10). Post-submission V1 planning in progress.

**Last updated:** 2026-05-19 (Phase 0 / R1 / Codama action items rolled back after disk verification — no Codama integration is present in any consumed code path).

---

## Mission

BracketChain is a trustless tournament protocol for PC esports on Solana. Players join real-money tournaments (Dota 2, CS2), play their matches offline, and receive automatic on-chain prize distribution. No off-chain custody, no payout delays, no admin override of match results.

**One-sentence product description:** *"Connect with Steam, join a Dota 2 tournament, play your match, get paid in USDC — without trusting an organizer."*

## Strategic priority

**Primary target market: B2C — community tournaments.**

Friend groups, streamer-organized events, community-tier competitive play. Tournaments are small (4-64 players), entry fees are low ($1-$10), prize pools modest ($4-$640). Players use phones or laptops; they don't have institutional wallet infrastructure.

**Why B2C first:**
- Faster validation cycle — community organizers iterate weekly; B2B contracts close yearly.
- Onboarding friction matters most at low stakes — if a $5 tournament requires Phantom setup, players bounce at 90%+.
- Phase 1-3 builds product-market signal for V3+ B2B expansion.
- Foundation features (settlement, identity, formats) are shared with B2B — work isn't wasted if B2B becomes priority later.

**Secondary: B2B — major esports organizations.** ESL, FACEIT, ESEA-tier orgs. Deferred to Phase 4. B2B-specific features (GameServer attestation, white-label, analytics tier) layer on the same foundation but ship later.

**Anti-priority: pure-crypto early adopters as primary target.** This audience adopts MVP-as-is and gives misleading signal. B2C-first deliberately prioritizes the harder onboarding path because it surfaces the *real* product gaps.

---

## Phase overview

| Phase | Theme | Duration (solo) | Plans included |
|---|---|---|---|
| **Phase 0** | Pre-Phase-1 foundation | ~1.5 weeks | [Phase 0 plan](./bracketchain-phase-0-foundation.md): solana-keychain + Codama pipeline |
| **Phase 1** | Real-money tournament foundation | ~12-15 weeks | V1.1 + V1 player-reported + V1.2 + V1 program-improvements + V1 partial-cancel |
| **Phase 2** | Onboarding unlock | ~8-10 weeks | V1 webapp C (Privy) + A (Dashboard) + B (Notifications) + V1 SDK hooks |
| **Phase 3** | Engagement & reach | ~10-12 weeks | V1 webapp D (Profile + Badges) + V2 dist A (Widget) + V2-A (Sponsor) + V1 formats A (RR) + Mobile UX |
| **Phase 4** | B2B & power-user (optional) | ~12-15 weeks | V2-C (GameServer) + V2 dist D (White-label) + V2 dist C (Analytics) + V1 formats B/C (DE/Swiss) + V2 dist B (Unity) |
| **Phase 5** | V3 research | TBD | ELO/rating, staked arbiter (not yet specced) |

**Total to B2C feature-complete (Phases 0-3): ~31.5-38.5 weeks solo, ~16-20 weeks team-of-2.**

---

## Phase 1 — Real-Money Tournament Foundation

**Goal:** A player authenticates via Steam, joins a USDC-entry Dota 2 / CS2 tournament, plays their match offline, reports the result trustlessly (with dispute window), and receives a prize payout automatically. **One single program redeploy** opens up real-money settlement.

### Single redeploy contains

| Plan | Key contribution |
|---|---|
| [V1.1 — Game Schema + SAS Identity](./bracketchain-v1.1-plan.md) | `SupportedGame` + `SettlementMode` enums; Steam OpenID → SAS attestation pipeline; `Participant.identity_hash` + foundation stats fields (`wins`, `losses`, `points_for`, `points_against`); `event_version: u8` convention locked across all events |
| [V1 — Player-Reported + VRF](./bracketchain-v1-player-reported-plan.md) | Proposal envelope on MatchNode (7 fields); Switchboard VRF (`request_seed` / `reveal_seed`); 6-ix dispute flow (propose / confirm / dispute / claim / resolve / force-claim); narrow notifications kernel; **MatchNode PDA seed schema-prep for formats** (`bracket: u8` byte added) |
| [V1.2 — Oracle Settlement](./bracketchain-v1.2-plan.md) | Switchboard On-Demand integration; `commit_match_lobby` + `bind_match_feed` + `propose_result_oracle`; oracle is just another proposer writing into V1's envelope (no parallel dispute system) |
| [V1 — Program Improvements](./bracketchain-v1-program-improvements-plan.md) | `PayoutPreset::Custom([u16; 8])` for arbitrary payout structures; `close_tournament` for rent reclaim (**B2C launch-critical** — without it, rent costs ~50% of pool for small tournaments) |
| [V1 — Partial Cancel](./bracketchain-v1-partial-cancel-plan.md) | `PartialCancelled` status + two-phase cancel + permissionless refund chunks; "make-whole survivors" policy |

### What ships in Phase 1 user-flow

1. **Organizer creates tournament** — picks game (Dota 2 / CS2 once V1.2 lands more games), settlement mode (`OrganizerOnly` / `PlayerReported` / `Oracle`), payout preset (Standard / Deep / WTA / Custom), entry fee, max participants.
2. **Player joins** — connects wallet (Phantom for now; Privy in Phase 2), links Steam via OpenID → indexer issues SAS attestation → SDK includes attestation account when calling `join_tournament` → vault credited.
3. **Organizer requests VRF seed** — `request_seed` ix commits to a future Switchboard randomness account.
4. **VRF reveal cron** (indexer-driven, permissionless) — `reveal_seed` runs in a later slot → `tournament.seed_hash` populated → `seed_revealed = true`.
5. **Organizer starts bracket** — `start_tournament` gated on `seed_revealed`; matches initialized via chunked ix calls.
6. **Players play matches offline** — actual Dota / CS gameplay on Steam matchmaking.
7. **Result reporting** — flow depends on settlement mode:
   - **PlayerReported:** Player A calls `propose_result` → 1-hour dispute window → Player B `confirm_result` or `dispute_result` → if no response, indexer's `auto-claim` cron calls `claim_result` at deadline.
   - **Oracle:** Organizer calls `commit_match_lobby` + `bind_match_feed` → Switchboard feed reads Steam Web API in TEE → relayer (anyone) calls `propose_result_oracle` → same dispute window flow as PlayerReported.
8. **Bracket advancement** — winner auto-advances to next match's `player_a` or `player_b` slot.
9. **Final match completes** — prize distribution auto-triggers: 96.5% to placements per chosen preset, 3.5% to protocol treasury.
10. **7-day grace** — players can view bracket on-chain.
11. **`close_tournament` cron** — indexer permissionlessly closes Tournament + MatchNode + Participant PDAs; organizer's wallet recovers rent.

### What does NOT ship in Phase 1

| Deferred to | Reason |
|---|---|
| Phase 2 — Privy social login | Biggest B2C onboarding unlock, deserves dedicated phase |
| Phase 3 — cNFT badges | Engagement loop, after basic flow validated |
| Phase 3 — Twitch widget | Distribution surface, after core product works |
| Phase 4 — DE / Swiss formats | Schema pre-shipped via cherry-pick; ix logic deferred (SingleElim covers 80%+ B2C use cases) |
| Phase 4 — GameServer attestation | B2B-specific (no community organizer has dedicated servers) |
| Phase 4 — White-label / Analytics tier | B2B revenue features |

### Pre-Phase 1 infrastructure prerequisites

Must complete before any V1.1 program code is written:

| Item | Purpose | Time |
|---|---|---|
| Codama-generated client pipeline | Eliminates manual IDL synchronization risk across SDK + indexer | 1-2 weeks |
| `IndexerKeyManager` service | Named roles for 5+ funded keypairs (SAS, claim, VRF, refund, cleanup) | 3-4 days |
| `PermissionlessDriver` cron abstraction | Consolidates auto-claim + VRF-reveal + partial-refund + close-terminal into one driver | 3-4 days |
| Switchboard On-Demand devnet queue access | VRF + Oracle feeds | 1-2 days |
| Steam Web API key | Steam OpenID + Oracle feed jobs | 1 day |
| OpenDota API key | Oracle cross-check for Dota 2 | 30 min |
| Funded keypairs (5x): SAS issuer, claim payer, VRF payer, refund payer, cleanup payer | Indexer-driven crons | 1 day (airdrops) |

### Phase 1 acceptance gates

- [ ] Full multi-wallet smoke (`OrganizerOnly` mode): create 4-player tournament, complete all matches via organizer, prize distributes correctly.
- [ ] `PlayerReported` smoke: 4 players join via Steam OpenID, players propose/confirm match results, bracket advances live via WebSocket.
- [ ] `Oracle` smoke (mocked feed): organizer commits + binds, mock feed returns winner hash, oracle proposal writes envelope, cron claims after deadline.
- [ ] Dispute resolution: PlayerReported match disputed, organizer-as-arbitrator calls `resolve_dispute`, bracket finalizes with correct winner.
- [ ] 24h force-claim: organizer ghosts a disputed match, anyone calls `force_claim_disputed` after 24h.
- [ ] Custom payout: tournament with `[50, 30, 20, 0, 0, 0, 0, 0]` distributes correctly.
- [ ] Rent reclaim: completed tournament → `close-terminal.cron` closes all accounts after 7 days → organizer wallet recovers ~95% of original rent.
- [ ] Partial-cancel: mid-tournament cancel returns funds per make-whole-survivors policy (survivors → full refund; eliminated → nothing; organizer → surplus).
- [ ] Event versioning: indexer rejects events with `event_version != EVENT_VERSION_V1` and emits `unknownEventVersion` metric.

---

## Phase 2 — Onboarding Unlock

**Goal:** Web2-grade onboarding. Email/social login, embedded wallets, push notifications. The phase that converts BracketChain from "crypto-only product" to "real product."

### Key plans

| Plan | Key contribution |
|---|---|
| [V1 Webapp Phase C — Privy](./bracketchain-v1-webapp-plan.md) | **Most important Phase 2 deliverable.** Privy embedded wallets via email/Google login + MoonPay/Coinbase Pay fiat ramp. User signs in with email → embedded Solana wallet auto-created → joins tournament without ever knowing what a wallet is. |
| [V1 Webapp Phase A — Dashboard + Discovery](./bracketchain-v1-webapp-plan.md) | Organizer dashboard ("my tournaments", analytics aggregates); tournament browser with prize/entry/status/sort filters; `/organizers/:wallet/analytics` endpoint. |
| [V1 Webapp Phase B — Notifications](./bracketchain-v1-webapp-plan.md) | Full unified notification event bus (extends V1 player-reported's narrow kernel). Web Push: `matchReady`, `payoutReceived`, `tournamentStarting`, `tournamentCancelled`, plus dispute events. Per-wallet preferences. Service Worker registration. |
| [V1 SDK Hooks](./bracketchain-v1-sdk-hooks-plan.md) | `@bracketchain/sdk/react` subpath + 3 hooks (`useTournament`, `useBracket`, `useEscrow`) + auto-resub on WebSocket disconnect (Drift v2 pattern). Eliminates "the bracket sometimes gets stuck" UX. |

**Strategic note:** Privy Phase C is **the** B2C unlock. Without it, BracketChain is "for crypto users who happen to play Dota." With it, it's "for Dota players who happen to want trustless prizes." That's an order-of-magnitude adoption difference.

### No program changes in Phase 2

Frontend + indexer + SDK only. The Phase 1 program redeploy stays as-is.

### Phase 2 acceptance gates

- [ ] Email login → embedded wallet → join tournament without installing Phantom.
- [ ] External-wallet (Phantom) login still works; optional email link for notifications.
- [ ] MoonPay flow: convert $10 USD → USDC delivered to embedded wallet in <5 minutes.
- [ ] Push notification fires within 5s of match becoming Active (counterparty available).
- [ ] `useTournament(pda)` hook re-renders bracket on live match-report tx confirmation.
- [ ] Pull-to-refresh on tournament page hits indexer (fast) with RPC fallback if stale.

---

## Phase 3 — Engagement & Reach

**Goal:** Build the viral loop. Returning players via badges/profile. New player acquisition via Twitch streamer embedding.

### Key plans

| Plan | Key contribution |
|---|---|
| [V1 Webapp Phase D — Profile + cNFT Badges](./bracketchain-v1-webapp-plan.md) | Public `/profile/[wallet]` route; aggregated stats from `player_stats` view; cNFT badges via Metaplex Bubblegum (`first_win`, `tournament_champion`, `won_5_tournaments`, etc.); tournament history; match-record chart. |
| [V2 Distribution Phase A — Widget](./bracketchain-v2-distribution-plan.md) | `@bracketchain/widget` npm package for React; Twitch panel embedding; <80KB gzipped; CSS-vars theming with 3 preset themes. Single `<BracketChainTournament pda={...} />` drop-in. |
| [V2-A — Sponsor (user-visible release)](./bracketchain-v2-plan.md) | Sponsor injection flow: any wallet can inject funds into Active tournament's vault with 5% protocol fee; sponsor leaderboard; `SponsorshipReceived` push notifications (depends on Phase 2's notification bus). |
| [V1 Formats Phase A — Round Robin](./bracketchain-v1-formats-plan.md) | Friend-group leagues, 16-player cap; deterministic Circle-method pairing; wins → head-to-head → differential tiebreaker. |
| **Mobile UX polish** | Responsive bracket view (horizontal scrolling, not grid), touch-first interaction, PWA installable, performance-budget audits. **Not in any individual plan — must be explicit Phase 3 deliverable.** |

### Phase 3 acceptance gates

- [ ] Player wins first match → `first_win` cNFT badge minted to their wallet within 5 minutes (`eligibility` cron + `minter` cron).
- [ ] Streamer embeds widget on Twitch panel → audience sees live bracket → audience members click "Join" → onboard via Privy.
- [ ] Sponsor injects $50 → prize pool grows by $47.50 → sponsor appears in tournament header → all participants get `payoutReceived` notifications when prizes distribute.
- [ ] RR tournament with 16 players → 120 matches reported → standings computed with all three tiebreakers.
- [ ] Mobile UX: full create-join-play flow works on iPhone Safari and Android Chrome.
- [ ] Widget bundle size <80KB gzipped (CI gate).

---

## Phase 4 — B2B & Power-User (Optional)

**Goal:** Enable major esports organizations (ESL, FACEIT, ESEA) to use BracketChain as their tournament infrastructure. Phases 1-3 are foundation; Phase 4 layers B2B-specific surface area.

**Activation trigger:** Phase 4 is **conditional** on concrete B2B interest. Do not proactively ship if Phase 1-3 hasn't surfaced demand.

### Key plans

| Plan | Key contribution |
|---|---|
| [V2-C — GameServer Attestation](./bracketchain-v2-plan.md) | Dedicated game servers attest match results via Ed25519 signatures; organizer registers allowlist of pubkeys per tournament; writes into V1's proposal envelope with `source = GameServer`. |
| [V2 Distribution Phase D — White-Label](./bracketchain-v2-distribution-plan.md) | Custom domain + branding + feature toggles per tenant; separate deployment per tenant (not multi-tenant routing); shared backend infrastructure. |
| [V2 Distribution Phase C — Analytics Tier](./bracketchain-v2-distribution-plan.md) | Pro+ tier with tournament-level dashboards, retention cohorts, revenue analytics, CSV export; tier-gating via wallet flag (`WalletTier` table); billing mechanism deferred to V2.1. |
| [V1 Formats Phase B — Double Elimination](./bracketchain-v1-formats-plan.md) | FGC + Smash + competitive CS scrim format; winners + losers + grand brackets via `bracket: u8` PDA seed (already shipped in Phase 1). |
| [V1 Formats Phase C — Swiss](./bracketchain-v1-formats-plan.md) | CS:GO Major format; per-round pairing using V1's VRF-revealed `seed_hash`; Buchholz tiebreaker. |
| [V2 Distribution Phase B — Unity SDK](./bracketchain-v2-distribution-plan.md) | C# wrapper for game developers; external browser wallet handoff; prefabs (BracketCanvas, TournamentCard). |

### Phase 4 acceptance gates (per plan, not bundled)

Phase 4 is the only phase where plans ship **independently** rather than bundled. Acceptance gates live in each plan; no aggregate gate.

---

## Phase 5 — V3 Research

| Item | Status |
|---|---|
| On-chain ELO / rating | Sketched in V2 plan's V3 Outlook; warrants dedicated plan |
| Staked arbiter system | Sketched in V2 plan's V3 Outlook; warrants dedicated plan |

Both have major architectural decisions (algorithm choice for ELO; governance model for arbiter overturns) that deserve their own planning conversations when ready to ship.

---

## Critical Path & Dependencies

```
                  ┌────────────────────────────────────┐
                  │ MVP (shipped 2026-05-10)           │
                  └────────────────┬───────────────────┘
                                   │
                                   ▼
       ┌─────────────────────────────────────────────────────┐
       │ PHASE 1 — Real-Money Tournament Foundation          │
       │ (single program redeploy — fresh program ID)        │
       │                                                     │
       │   V1.1 ─────► V1 player-reported ─────► V1.2        │
       │   (foundation) (settlement engine)    (oracle)      │
       │       │                │                            │
       │       └────────────────┴───────► V1 program-improv  │
       │                                  V1 partial-cancel  │
       │                                                     │
       │   = ONE anchor deploy → new program ID              │
       └────────────────┬────────────────────────────────────┘
                        │ ~12-15 weeks solo
                        ▼
       ┌─────────────────────────────────────────────────────┐
       │ PHASE 2 — Onboarding Unlock                         │
       │   V1 webapp C (Privy) ◄── biggest B2C deliverable   │
       │ + V1 webapp A (Dashboard)                           │
       │ + V1 webapp B (Notifications full event bus)        │
       │ + V1 SDK hooks                                      │
       │                                                     │
       │   = no program changes; frontend + indexer + SDK    │
       └────────────────┬────────────────────────────────────┘
                        │ ~8-10 weeks
                        ▼
       ┌─────────────────────────────────────────────────────┐
       │ PHASE 3 — Engagement & Reach                        │
       │   V1 webapp D (Profile + cNFT badges)               │
       │ + V2 dist A (Widget for Twitch embedding)           │
       │ + V2-A user release (Sponsor flow)                  │
       │ + V1 formats A (Round Robin)                        │
       │ + Mobile UX polish (no plan; explicit deliverable)  │
       └────────────────┬────────────────────────────────────┘
                        │ ~10-12 weeks
                        ▼
            ★ ~30-37 weeks total to B2C feature-complete ★
                        │
                        │ (Phase 4 activation = OPTIONAL —
                        │  conditional on B2B demand signal)
                        ▼
       ┌─────────────────────────────────────────────────────┐
       │ PHASE 4 — B2B & Power-User (OPTIONAL)               │
       │   V2-C (GameServer attestation)                     │
       │   V2 dist D (White-label)                           │
       │   V2 dist C (Analytics tier)                        │
       │   V1 formats B (DE) + C (Swiss)                     │
       │   V2 dist B (Unity SDK)                             │
       └────────────────┬────────────────────────────────────┘
                        │ ~12-15 weeks if pursued
                        ▼
       ┌─────────────────────────────────────────────────────┐
       │ PHASE 5 — V3 Research                               │
       │   ELO / rating + staked arbiter — future plans      │
       └─────────────────────────────────────────────────────┘
```

### Key sequencing constraints (locked)

These ordering rules are **immutable**. If you find yourself wanting to violate one, update this roadmap first with rationale.

| # | Constraint | Source plan |
|---|---|---|
| C1 | V1.1 schema-prep must ship before V1 player-reported (identity, stats fields, `event_version`, Hybrid drop) | V1.1 plan |
| C2 | V1 player-reported must ship before V1.2 (oracle reuses V1's proposal envelope) | V1.2 plan prerequisite |
| C3 | V1 player-reported must ship before V1 formats Phase C (Swiss needs VRF for pairing entropy) | V1 formats plan |
| C4 | V1 player-reported must ship before V2-C (GameServer reuses envelope) | V2 plan |
| C5 | V1 webapp Phase B (Notifications) must ship before V2-A user-visible (sponsor needs push delivery) | V1 webapp plan, V2 plan |
| C6 | V1 webapp Phase A must ship before V2 distribution Phase C (analytics extends Phase A endpoint) | V2 distribution plan |
| C7 | V1 program-improvements `close_tournament` must ship in Phase 1 (rent economics make B2C non-viable otherwise) | V1 program-improvements plan |
| C8 | `Participant.wins/losses/points_*` ship in V1.1, not formats plan (partial-cancel + formats + webapp Phase D all consume) | V1.1 plan |
| C9 | MatchNode PDA seed `bracket: u8` ships in V1 player-reported (avoids second redeploy for formats Phase A) | V1 player-reported plan |
| C10 | All `#[event]` structs have `event_version: u8` as first field (silent decode prevention) | V1.1 plan |

---

## Ownership-Locked Decisions Index

Decisions that have been **explicitly locked** with single-source-of-truth ownership in a plan, with cross-references elsewhere. Don't relitigate without updating the owning plan first.

| Decision | Owner section | Status |
|---|---|---|
| Drop `Hybrid` variant from `SettlementMode` | V1.1 → Scope decisions | ✅ Locked |
| VRF lives in V1 player-reported, NOT V1.2 | V1 player-reported → Sequencing constraint | ✅ Locked |
| Custom payouts ship in V1 program-improvements (V2 Phase B = cross-ref only) | V1 program-improvements → Phase A Ownership | ✅ Locked |
| Notifications full event bus owned by V1 webapp Phase B (V1 player-reported = narrow kernel only) | V1 webapp plan → Phase B Ownership | ✅ Locked |
| `Participant.wins/losses/points_*` ship in V1.1, not formats Phase A | V1.1 → Modify: state/participant.rs | ✅ Locked |
| MatchNode PDA seed `bracket: u8` schema-prep ships in V1 player-reported, not formats Phase A | V1 player-reported → Modify: state/match_node.rs | ✅ Locked |
| `close_tournament` is B2C launch-critical, not optional polish | V1 program-improvements → Phase B Ownership | ✅ Locked |
| `event_version: u8` first field on every `#[event]` struct | V1.1 → Scope decisions | ✅ Locked |
| B2C primary, B2B secondary (Phase 4 optional) | This roadmap → Strategic priority | ✅ Locked |
| Single coordinated program redeploy for Phase 1 (no incremental V1 deploys) | This roadmap → Phase 1 | ✅ Locked |

---

## Risk Register

Top remaining risks active in Phase 1-2 work, with mitigations and status. **Phase 3-4 risks** are parked in [`bracketchain-deferred-risks.md`](./bracketchain-deferred-risks.md) and lifted into this register only when their phase activates.

| # | Risk | Mitigation | Status |
|---|---|---|---|
| R1 | Silent IDL drift between program / SDK / indexer | Codama-generated client + CI gate on IDL hash + `event_version: u8` byte check | **Not yet implemented — verified 2026-05-19 by disk inspection.** `bracket-chain-programs/codama.json` absent; SDK has no `src/generated/`; SDK still imports `@coral-xyz/anchor` in 6 files; SDK's vendored `src/idl/bracket_chain.json` still present. Indexer has an orphaned `src/generated/src/generated/` tree, but no app code imports it. Earlier "Mostly resolved" status was rolled back. `event_version: u8` planned for V1.1 redeploy. CI gate design (cross-repo sibling checkout) still unresolved. |
| R2 | Cron sprawl (5+ crons in Phase 1) | `PermissionlessDriver` consolidation pattern | Not yet implemented (pre-Phase-1 prerequisite) |
| R3 | Funded keypair sprawl (5+ keys in Phase 1) | `IndexerKeyManager` service with named roles | Deferred to Phase 1 — `solana-keychain` adoption (Phase 0 Section 1) moved to "do during Phase 1 cron build" 2026-05-19. Rationale: MVP indexer has zero signing paths to migrate today (reconciliation cron is read-only); Section 1's smoke test is non-applicable. Section 1 lands when the first signing cron (V1 auto-claim / VRF-reveal / partial-refund / close-terminal) is built. |
| R6 | Mobile UX work not in any plan | Explicit Phase 3 frontend deliverable | Acknowledged in this roadmap |
| R7 | Phase 1 timeline (12-15 weeks solo) | Team-of-2 cuts to ~6-8 weeks | Resource-dependent |
| R8 | Privy app config (recovery, security) | Resolve before Phase 2 begins | Pre-Phase-2 |
| R9 | Switchboard feed cost monitoring (Phase 1 V1.2) | `/health` metric for feed-creation count | Pre-Phase-1 ops setup |
| R10 | Rent economics for small B2C tournaments | `close_tournament` in Phase 1 (locked) | ✅ Locked |
| R11 | Frontend README stale (claims simulated transactions, SDK actually fully wired) | Update README before Phase 1 work begins | Pre-Phase-1 hygiene |
| R12 | Indexer `.env.example` `SOLANA_RPC_URL` vs `RPC_URL` bug | Renamed to `RPC_URL`; dead `SOLANA_CLUSTER` + `HELIUS_API_KEY` entries removed; README warnings cleaned | ✅ Resolved 2026-05-16 |

### Deferred to later phases

These risks are real concerns but **do not affect Phase 1-2 work**. Lifted into the register above when their activating phase begins. See [`bracketchain-deferred-risks.md`](./bracketchain-deferred-risks.md) for full mitigation analysis.

| # | Risk | Activates at |
|---|---|---|
| D1 | Sponsor refund-on-cancel griefing surface | Phase 3 — V2-A user-visible sponsor flow |
| D2 | Game-server key compromise mid-tournament (no `revoke_game_server` ix) | Phase 4 — V2-C implementation |
| D3 | Repo proliferation pressure on cross-IDL sync | Phase 4 — V2-distribution Phase B (Unity SDK) |

---

## Plan Inventory

All 11 plans, with their phase mapping, key contribution, and status.

| # | Plan | Phase | Status | Key contribution |
|---|---|---|---|---|
| 1 | [MVP context](./bracketchain-phase-0-foundation.md#mvp-context--what-shipped-2026-05-10) | Phase 0 | ✅ Shipped 2026-05-10 | 6-ix Anchor program; 3 payout presets; 2-128 participants; SDK 0.3.1 on npm; indexer on Railway. **Plan file `bracketchain-mvp-plan.md` deleted 2026-05-19** — summary (devnet IDs, deploy txs, surface) preserved in Phase 0 doc's MVP context section; original recoverable via `git log --all -- bracketchain-mvp-plan.md`. |
| 2 | [V1.1 — Game schema + SAS identity](./bracketchain-v1.1-plan.md) | Phase 1 | Spec | Foundation for game identity + settlement mode enums; `event_version` convention; Participant stats cherry-picked here |
| 3 | [V1 — Player-reported + VRF](./bracketchain-v1-player-reported-plan.md) | Phase 1 | Spec | Settlement engine: proposal envelope + VRF + dispute flow; MatchNode PDA seed schema-prep; narrow notifications kernel |
| 4 | [V1.2 — Oracle settlement](./bracketchain-v1.2-plan.md) | Phase 1 | Spec | Switchboard On-Demand integration; oracle writes V1's envelope; match-ID commitment for unredirectable feeds |
| 5 | [V1 — Program improvements](./bracketchain-v1-program-improvements-plan.md) | Phase 1 | Spec | Custom payouts (`PayoutPreset::Custom`) + `close_tournament` for rent reclaim (B2C launch-critical) |
| 6 | [V1 — Partial cancel](./bracketchain-v1-partial-cancel-plan.md) | Phase 1 | Spec | Mid-tournament cancel with make-whole-survivors policy; new `PartialCancelled` status |
| 7 | [V1 — Webapp (Phases A, B, C, D)](./bracketchain-v1-webapp-plan.md) | Phase 2 (A, B, C); Phase 3 (D) | Spec | A: Dashboard/Discovery; B: Notifications (owned here); C: Privy social login (biggest B2C unlock); D: Profile + cNFT badges |
| 8 | [V1 — SDK hooks](./bracketchain-v1-sdk-hooks-plan.md) | Phase 2 | Spec | `@bracketchain/sdk/react` subpath + auto-resub on WebSocket disconnect |
| 9 | [V1 — Formats (Phases A, B, C)](./bracketchain-v1-formats-plan.md) | Phase 3 (A); Phase 4 (B, C) | Spec | A: Round Robin (friend leagues); B: Double Elim (FGC); C: Swiss (CS Majors) — all schema pre-shipped in V1, this plan = pure ix logic |
| 10 | [V2 — Sponsor + GameServer](./bracketchain-v2-plan.md) | Phase 3 (V2-A); Phase 4 (V2-C) | Spec | V2-A: trustless sponsor injection; V2-C: game-server-direct attestation (B2B-specific) |
| 11 | [V2 — Distribution (Widget, Unity, Analytics, White-label)](./bracketchain-v2-distribution-plan.md) | Phase 3 (Widget); Phase 4 (Unity, Analytics, White-label) | Spec | Widget for Twitch embedding; Unity SDK; Pro/Enterprise analytics tier; white-label deployments |

---

## Glossary

Key terms used across plans. New contributors start here.

| Term | Meaning |
|---|---|
| **MatchNode** | On-chain account representing one match in a tournament's bracket. ~120 bytes MVP, ~340 bytes after Phase 1 redeploy. |
| **PDA (Program Derived Address)** | Deterministically-computed Solana account address from seeds + program ID. No keypair needed; program "knows" the address from formula. |
| **Settlement Mode** | Per-tournament policy for who reports match results: `OrganizerOnly`, `PlayerReported`, `Oracle`, `GameServer`. Locked at create-time. |
| **Proposal Source** | Per-match identifier of who proposed the result: `None`, `Player`, `Oracle`, `GameServer`. Lives in MatchNode's proposal envelope. |
| **Proposal Envelope** | The 7-field block on MatchNode (`proposal_source`, `proposer`, `proposed_winner`, `proposed_at`, `claim_deadline`, `disputed`, `dispute_reason`) that any settlement mode writes into. **Load-bearing abstraction across V1 / V1.2 / V2-C.** |
| **VRF (Verifiable Random Function)** | Cryptographic primitive that produces verifiable pseudo-randomness with proof that the output came from a committed input. BracketChain uses Switchboard On-Demand's VRF for unmanipulable bracket seeding. |
| **SAS (Solana Attestation Service)** | Composable on-chain attestation framework. BracketChain uses it to bind wallet ↔ Steam ID via Steam OpenID. Program ID: `22zoJMtdu4tQc2PzL74ZUT7FrwgB1Udec8DdW4yw4BdG`. |
| **Switchboard On-Demand** | Solana oracle / VRF infrastructure. BracketChain uses it for VRF (Phase 1) and oracle settlement (Phase 1, V1.2). Devnet program ID: `Aio4gaXjXzJNVLtzwtNVmSqGKpANtXhybbkhtAC94ji2`. |
| **Codama** | Tool to generate TypeScript/C# clients from Solana program IDLs. Eliminates manual IDL synchronization risk. Pre-Phase-1 prerequisite. |
| **Borsh** | Binary serialization format used by Anchor. **Positional** — adding fields in the middle of a struct breaks all decoders without explicit versioning. |
| **Anchor** | Solana program framework. BracketChain pinned to 0.32.1. |
| **`event_version: u8`** | First-field convention on every `#[event]` struct. Indexer parser rejects mismatched versions to prevent silent decode. Initial value `EVENT_VERSION_V1 = 1`. |
| **Permissionless cleanup** | Pattern where organizer drives happy path but any wallet can drive recovery (`claim_result`, `partial_refund_chunk`, `close_tournament`, `reveal_seed`). Prevents organizer-griefing. |
| **Privy** | Embedded-wallet auth provider. Lets users sign in via email/Google → auto-creates Solana wallet. Plus MoonPay/Coinbase Pay fiat on-ramp. |
| **Bubblegum** | Metaplex compressed-NFT (cNFT) framework. Used in Phase 3 for badge minting at ~$0.001/badge mainnet cost. |
| **Helius DAS** | Digital Asset Standard API for reading cNFTs. Phase 3 frontend uses this to display earned badges on profile pages. |

---

## Action Items

Concrete next steps, sorted by sequence.

### Before any V1.1 program code is written (Pre-Phase-1)

- [ ] **Confirm B2C-first strategy** with stakeholders (this roadmap is the artifact to confirm against).
- [x] ~~**Indexer hygiene**: Fix `.env.example` `SOLANA_RPC_URL` → `RPC_URL`~~ — ✅ Resolved 2026-05-16 (R12).
- [ ] **Frontend hygiene**: Update `BracketChain-Frontend/README.md` to reflect SDK 0.3.0 wiring (30 min, R11) — **also tracked as Phase 0 Section 3.5**.

**Phase 0 — see [`bracketchain-phase-0-foundation.md`](./bracketchain-phase-0-foundation.md):**

- [~] **solana-keychain adoption** (Section 1) — **deferred to Phase 1 cron build** 2026-05-19. MVP indexer has no signing path to migrate today; Section 1 lands when the first signing cron is built (V1 auto-claim / VRF-reveal / partial-refund / close-terminal).
- [ ] **Codama pipeline setup** (Section 2 / Stage 1+2) — **Not started — rolled back 2026-05-19.** Earlier "Shipped 2026-05-18" status was not reflected in the working tree (no `codama.json` in `bracket-chain-programs/`; no `src/generated/` in SDK; SDK still on `@coral-xyz/anchor`). The Makefile `codama-generate` target exists but fails until `codama.json` is written. Indexer has an orphaned `src/generated/src/generated/` tree but no app code imports it. See not-shipped banners on the 2026-05-18 / 2026-05-19 execution logs in the Phase 0 doc.
- [ ] **CI gate on generated client drift** — not started; cross-repo CI checkout design problem remains; local regen discipline is the fallback when Codama is actually adopted.
- [ ] **Stage 3 — SDK Kit migration** — **Not started — rolled back 2026-05-19.** Earlier "Effectively complete" status not reflected in the working tree; SDK's `src/methods/*`, `src/errors.ts`, `src/index.ts`, `src/client.ts`, `src/types.ts` all still import from `@coral-xyz/anchor`. The 2026-05-18 / 2026-05-19 execution logs preserve useful gotcha-findings for a future attempt.
- [ ] **Stage 4 — frontend boundary via `@solana/web3-compat`** — not started.
- [ ] **Scripts migration** — `scripts/init-protocol.ts` + `scripts/e2e-demo.ts` both still on legacy Anchor path (which is fine — SDK is still on Anchor too).
- [ ] **Phase 0 acceptance gates** — all unchecked; gate before Phase 1.

**Phase 0 Section 3 — MVP gap closure (added 2026-05-19):**

- [ ] **Section 3.1** — Indexer webhook HMAC auth (`HELIUS_WEBHOOK_SECRET` wired with raw-body middleware + signature guard; ~3-4h).
- [ ] **Section 3.2** — Indexer test coverage: all 7 event handlers (happy-path + re-delivery) + reconciliation drift cases + Task #20 `test-parser.mjs` payload fix (~1-2 days).
- [ ] **Section 3.3** — Indexer `GET /tournaments/check-name` endpoint; frontend `DuplicateNameWarning` mock replaced (~2-3h indexer-side; small frontend follow-up).
- [ ] **Section 3.4** — Anchor Tier-4 tests: `organizer-deposit.test.ts` (3 tests) + `capacity-128p-deep.test.ts` (CU measurement) + `CU_BUDGET.md` baseline (~1-2 days).
- [ ] **Section 3.5** — Frontend README rewrite (R11; 30 min) — duplicate of pre-Phase-1 hygiene item above.

**Phase 1 pre-flight (parallel with or after Phase 0):**

- [ ] **Switchboard On-Demand devnet queue access** — apply, fund payer keypair, smoke-test sample randomness flow (1-2 days).
- [ ] **Steam Web API key** — register, test sample `GetMatchHistory` (1 hour).
- [ ] **Solana Attestation Service onboarding** — write throwaway test creating Credential + Schema + Attestation (1-2 days).
- [ ] **`PermissionlessDriver` cron abstraction** (R2) — designed and built **during Phase 1**, not Phase 0; as 4 new cron services land, they register via the abstraction (Helium config-driven + Drift production-guards pattern).

### Phase 1 program work (sequential within Anchor codebase)

- [ ] V1.1 state additions (`state/game.rs`, Tournament/Participant extensions) + `set_sas_config` ix + `join_tournament` SAS validation block.
- [ ] V1 player-reported state additions (proposal envelope on MatchNode + schema-prep `bracket: u8` PDA seed change + VRF fields).
- [ ] V1 player-reported ix: `request_seed`, `reveal_seed`, `propose_result`, `confirm_result`, `dispute_result`, `claim_result`, `resolve_dispute`, `force_claim_disputed`, `migrate_v1_tournament`.
- [ ] V1.2 state additions (commitment, switchboard_feed, arbitrator) + ix: `commit_match_lobby`, `bind_match_feed`, `propose_result_oracle`, `set_oracle_config`.
- [ ] V1 program-improvements: `PayoutPreset::Custom` validation + `close_tournament` ix.
- [ ] V1 partial-cancel: `partial_cancel_tournament` + `partial_refund_chunk` + `PartialCancelled` status variant.
- [ ] Cross-plan signer rule updates: `dispute_result` gains Oracle source arm.
- [ ] `event_version: u8` first field on all `#[event]` structs.
- [ ] Comprehensive Anchor mocha test suite covering all settlement modes + edge cases + Phase 1 acceptance gates.

### Phase 1 single redeploy ceremony

- [ ] `anchor build` → verify clean compile with all V1 additions.
- [ ] `anchor deploy --provider.cluster devnet` → new program ID; record in `bracketchain-main/README.md`.
- [ ] `make sync-idl` → SDK + indexer vendored IDL copies.
- [ ] Codama regenerate (automatic from new IDL).
- [ ] SDK bump to `0.4.0`, publish to npm.
- [ ] Indexer `prisma migrate deploy` (new tables + enum extensions).
- [ ] Bootstrap: `initialize_protocol`, `set_sas_config`, `set_oracle_config`, SAS Credential + Schema creation via indexer boot.
- [ ] Smoke-test all 9 Phase 1 acceptance gates (above).
- [ ] Announce on community channels.

### Post-Phase 1 (transition to Phase 2)

- [ ] Update this roadmap document with Phase 1 completion status.
- [ ] Begin Phase 2 — Privy app config (R8 resolution) → V1 webapp C implementation.

---

## Update Protocol

**This file is updated _before_ plan files when priorities shift.** Per-plan changes follow this roadmap, not the other way around.

**Editing rules:**
1. **Priority/scope changes** — update Strategic priority section, then propagate to affected phase sections.
2. **New ownership-locked decisions** — add to Ownership-Locked Decisions Index, then update the owning plan with the canonical decision text.
3. **New risks** — add to Risk Register with mitigation + status. Status options: `Acknowledged`, `Mitigated`, `Locked`, `Resolved`, `Deferred`.
4. **Plan additions** — add to Plan Inventory, then create the plan file at root level (`bracketchain-{phase}-{topic}-plan.md`).
5. **Phase rebalancing** — update Phase overview table, affected phase deep-dive sections, and Critical Path graph.

**Anti-patterns to avoid:**
- Updating plan files for a sequencing decision without updating this roadmap first (creates inconsistency).
- Adding decision rationale here that contradicts a plan's "Ownership (locked)" section (the plan wins; sync this file).
- Letting risk register grow without status updates (stale risks erode trust in the register).
- Treating Phase 4/5 as binding — they're aspirational and may be reordered/cut as Phase 1-3 surfaces real signal.
