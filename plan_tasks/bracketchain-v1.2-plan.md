# BracketChain V1.2 — Phase 2: Switchboard Oracle Settlement

## Context

**Prerequisite — V1 player-reported plan (`bracketchain-v1-player-reported-plan.md`) must ship first.** V1 introduces the generic proposal/dispute primitive on `MatchNode` (`proposal_source`, `proposer`, `proposed_winner`, `proposed_at`, `claim_deadline`, `disputed`, `dispute_reason`), the instruction set that drives it (`propose_result`, `confirm_result`, `dispute_result`, `claim_result`, `resolve_dispute`, `force_claim_disputed`), **and Switchboard VRF for bracket seeding** (`request_seed` / `reveal_seed`, plus the `tournament.seed_hash` / `seed_revealed` fields). V1 reserves `ProposalSource::Oracle` in its enum — this plan flips that variant from reserved to live.

V1.1 lands earlier still: schema and identity (`SupportedGame`, `SettlementMode` with `OrganizerOnly | PlayerReported | Oracle` — `Hybrid` dropped in V1.1's cleanup pass; see that plan — `Participant.identity_hash`, SAS attestations).

This plan adds, on top of both:
1. **Switchboard On-Demand pull feeds** for verified match-result delivery, with TEE-protected Steam Web API access.
2. **Match-ID commitment** on `MatchNode` so an attacker can't redirect the oracle to a different match.
3. **One new instruction — `propose_result_oracle`** — that reads a bound Switchboard feed and writes into V1's proposal envelope.
4. **One signer-rule broadening on V1's `dispute_result`** so an arbitrator (or either player) can dispute oracle proposals.

VRF (the V1 primitive) is reused unchanged here — Oracle-mode tournaments inherit `settlement_mode != OrganizerOnly` gating, so they automatically require VRF-revealed seeding. No additional VRF surface in this plan.

**VRF ownership is locked in V1 player-reported plan.** If you are tempted to lift `request_seed` / `reveal_seed` into V1.2 because Oracle features are landing first — don't. The sequencing constraint is documented in V1 player-reported's `Sequencing constraint (locked)` section: VRF must ship with V1 (the moment real money enters the system), not with V1.2 (which is incremental on top). Lifting VRF here would either duplicate the primitive (wire-breaking when V1 lands) or push formats Phase C (Swiss) and V2-C (GameServer) to wait for V1.2 redeploy. Either path is more expensive than respecting the dependency order: **V1.1 → V1 player-reported (with VRF) → V1.2 (consumes VRF).**

**Game scope:** Dota 2 only. CS2 / Valorant / LoL stay reserved in V1.1's `SupportedGame` enum and ship in V1.3+.

**Settlement scope:** match-end winner only. No BR placement payouts (different `MatchOutcome` envelope — separate plan).

---

## The thin layer — what V1 gives us vs. what V1.2 adds

The architectural payoff of sequencing V1 first (incl. VRF) is that V1.2's instruction surface collapses from 9 ixs in the original draft to **4 new** + **1 modified**.

### V1.2 reuses unchanged from V1

| V1 surface                  | Role in V1.2                                                              |
|-----------------------------|---------------------------------------------------------------------------|
| `request_seed` / `reveal_seed` | **VRF inherited as-is.** Oracle-mode tournaments are `settlement_mode != OrganizerOnly`, so V1's existing VRF gate fires automatically. |
| `confirm_result`            | Not used by oracle path (oracle proposals don't need a counterparty confirm — the dispute window + permissionless claim handle it). |
| `claim_result`              | **Used identically.** After the dispute window closes on an oracle-proposed match, anyone (or the indexer's existing cron) calls `claim_result` to finalize. |
| `dispute_result`            | **Used with one signer-rule extension** (see below).                      |
| `resolve_dispute`           | Used identically when an arbitrator overrides a disputed oracle result.   |
| `force_claim_disputed`      | Used identically as the 24h-organizer-silence safety valve.               |
| `report_result`             | Rejected for Oracle mode (already rejected for PlayerReported in V1).     |

### V1.2 adds

| New / modified                  | Why                                                          |
|---------------------------------|--------------------------------------------------------------|
| `commit_match_lobby` (new)      | Organizer commits the lobby ID + player identities before launching the Dota lobby. |
| `bind_match_feed` (new)         | Organizer binds a Switchboard PullFeed account to the match. |
| `propose_result_oracle` (new)   | Permissionless. Reads the bound feed, writes V1's proposal envelope with `source = Oracle`. |
| `set_oracle_config` (new)       | Admin: Switchboard program/queue/staleness config.           |
| `migrate_v1_2_tournament` (new) | Reallocs Tournament + MatchNode for new fields.              |
| `dispute_result` (modified)     | Signer broadens: counterparty is required for `source == Player`; either player OR `tournament.arbitrator` for `source == Oracle`. |

That's it on the program side. The frontend's V1 `ReportResultModal` action-dispatcher already routes propose/confirm/dispute/claim/resolve panels by viewer + match state — V1.2 only adds an "Oracle pending" badge variant plus the feed-binding modal.

---

## Architecture

```
                ┌─────────────────────────────────────────────────────┐
                │  Switchboard On-Demand (Solana program)             │
                │  Devnet program: Aio4gaXjXzJNVLtzwtNVmSqGKpANtXhybbkhtAC94ji2
                │                                                     │
                │  PullFeed PDA per tournament-match                  │
                │    └─ jobs: [steam_api_job, opendota_job]           │
                │    └─ min_job_responses = 2  (both must agree)      │
                │  (Randomness PDA / VRF — already wired by V1)       │
                └─────────────────────────────────────────────────────┘
                          ▲                                  ▲
                          │ create+update                    │ read
                          │ (TS client)                      │ (program)
                          │                                  │
                ┌─────────┴────────┐  ┌────────────────┐  ┌──┴───────────────────────────┐
                │ Indexer (Nest)   │  │ Frontend       │  │ BracketChain program          │
                │ /feeds/*         │  │ FeedBind +     │  │ commit_match_lobby            │
                │ - feed factory   │  │ OraclePending  │  │ bind_match_feed               │
                │ - existing V1    │  │ panels         │  │ propose_result_oracle ┐       │
                │   notifications  │  │ + existing V1  │  │   ↓ writes envelope    │      │
                │   + auto-claim   │  │ propose/       │  │ (V1) confirm_result    │ rest │
                │   cron           │  │ confirm/       │  │ (V1) claim_result      │ from │
                │ - existing V1    │  │ dispute/       │  │ (V1) dispute_result    │ V1   │
                │   VRF cron       │  │ claim/         │  │ (V1) resolve_dispute   │      │
                └──────────────────┘  │ resolve panels │  │ (V1) force_claim_dispd ┘      │
                                      └────────────────┘  │ (V1) start_tournament         │
                                                          │ (V1) request_seed/reveal_seed │
                                                          └───────────────────────────────┘
```

---

## Match-ID commitment design

(Unchanged from the original V1.2 draft — this is V1.2's contribution to the security model.)

The single most important security property in this plan: **the oracle cannot be redirected**. Without on-chain commitment, an attacker who controls the feed binding can point the Switchboard feed at a different Dota match where their preferred player happened to win.

### Why a single `external_match_id` is not enough

The Steam `match_id` is only assigned *after* the lobby launches. Committing it on-chain pre-match is impossible. Committing it post-match is trivially manipulable.

### `MatchCommitment` struct (new state file)

```rust
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, InitSpace)]
pub struct MatchCommitment {
    pub lobby_id: [u8; 16],          // organizer-chosen pre-match identifier
    pub player_a_game_id: [u8; 32],  // keccak(steam_id_64) — from participant.identity_hash
    pub player_b_game_id: [u8; 32],  // same
    pub committed_at: i64,
    pub committed_slot: u64,
}
```

The Switchboard `OracleJob` is parameterized at feed-creation time with `lobby_id` + both `player_a_game_id` and `player_b_game_id`. The job queries Steam's `GetMatchHistory` filtered to those two Steam IDs in the window after `committed_at`, validates the lobby ID matches, then returns the winning Steam ID. `propose_result_oracle` verifies the returned ID hashes to either `player_a_game_id` or `player_b_game_id`.

**This is why V1.1's `identity_hash` field is load-bearing.** Without it, the program has no on-chain truth to compare against — it would have to trust whatever Steam ID the oracle returned.

`★ Insight ─────────────────────────────────────`
`lobby_id` doesn't need to be the actual Dota 2 lobby ID — it can be any 16-byte value the organizer commits before launching the lobby (e.g., a hash of the bracket position + tournament + timestamp). What matters is that it's unforgeable post-hoc. The Steam side just needs to confirm "yes, these two Steam IDs played a match after this slot." The lobby ID is the human-side ceremony that connects "this on-chain match" to "go play now."
`─────────────────────────────────────────────────`

---

## Program changes (`bracket-chain-programs/`)

### Modify: `state/tournament.rs`

Add (after V1's VRF fields):

```rust
pub arbitrator: Pubkey,                  // defaults to organizer; Squads multisig later
```

Account-size cost: +32 bytes per Tournament. (`vrf_randomness_account`, `vrf_commit_slot`, `seed_revealed`, and `dispute_window_secs` are all V1's already.)

### Modify: `state/match_node.rs`

V1 already adds the full proposal envelope. V1.2 adds only:

```rust
pub commitment: Option<MatchCommitment>,    // None for non-Oracle tournaments
pub switchboard_feed: Pubkey,                // PullFeedAccountData PDA; default zero
```

Account-size cost: +97 (`MatchCommitment` InitSpace) +32 = 129 bytes per match. With ~32 matches in a 64-player bracket, ~4.1KB extra per tournament — fine.

### New file: `state/match_commitment.rs`

The `MatchCommitment` struct shown above. Re-export in `state/mod.rs`.

### Modify: `state/protocol_config.rs`

Add:

```rust
pub switchboard_program: Pubkey,          // Aio4...4ji2 on devnet
pub switchboard_queue: Pubkey,            // shared devnet queue
pub max_stale_slots: u32,                 // default 100
pub min_oracle_samples: u32,              // default 5
```

Set via a new `set_oracle_config` ix mirroring V1.1's `set_sas_config`.

### VRF + `start_tournament` — inherited from V1

V1 already ships `request_seed`, `reveal_seed`, the `tournament.seed_hash` / `seed_revealed` machinery, and the `start_tournament` gate that fires for any tournament with `settlement_mode != OrganizerOnly`. Oracle-mode tournaments (`settlement_mode == Oracle`) are by definition non-OrganizerOnly, so they inherit the VRF requirement automatically — no V1.2 work needed on either ix or on bracket seeding.

### New ix: `instructions/commit_match_lobby.rs`

Organizer-signed. Inputs: `round`, `match_index`, `lobby_id: [u8; 16]`. Logic:
- Tournament status = `Active`, match status = `Active`.
- `tournament.settlement_mode == Oracle` (otherwise `BadSettlementMode`).
- `player_a_game_id` and `player_b_game_id` sourced from each player's `Participant.identity_hash` (program looks them up, no client input).
- Sets `match.commitment = Some(MatchCommitment { ... })`.
- Idempotency: re-committing the same match → `MatchAlreadyCommitted`.
- Emits `MatchLobbyCommitted`.

### New ix: `instructions/bind_match_feed.rs`

Organizer-signed. Inputs: `round`, `match_index`, `switchboard_feed: Pubkey`. Logic:
- Match must already be committed (`commitment.is_some()`).
- Validates `switchboard_feed.owner == SWITCHBOARD_ON_DEMAND_PROGRAM_ID`.
- Optionally parses `PullFeedAccountData` to sanity-check the queue matches `protocol_config.switchboard_queue`.
- Sets `match.switchboard_feed = switchboard_feed`.
- Emits `MatchFeedBound`.

Why a separate ix from `commit_match_lobby`? Feed creation is a multi-step TS-side flow (queue, jobs, fund). Decoupling commit (cheap, fast, before the lobby launches) from binding (heavyweight, after the feed account exists) keeps the organizer flow incremental.

### New ix: `instructions/propose_result_oracle.rs`

**Permissionless** — anyone can submit it; trust bottoms out in the feed account contents. Inputs: `round`, `match_index`. Accounts:
- `tournament` (mutable)
- `match_account` (mutable)
- `participant_a`, `participant_b` (immutable; for identity_hash lookup if not already cached in commitment)
- `switchboard_feed` (immutable, the one bound earlier)
- `protocol_config`

Handler logic:

```rust
require!(tournament.settlement_mode == SettlementMode::Oracle, BadSettlementMode);
require!(match_account.status == MatchStatus::Active, MatchNotActive);
require!(match_account.proposal_source == ProposalSource::None, AlreadyProposed);
let commitment = match_account.commitment.ok_or(MatchNotCommitted)?;

// 1. Feed account is the one bound to this match
require_keys_eq!(*ctx.accounts.switchboard_feed.key, match_account.switchboard_feed,
    WrongFeedAccount);

// 2. Parse feed; enforce freshness
let feed = PullFeedAccountData::parse(ctx.accounts.switchboard_feed.data.borrow())?;
let winner_id_value = feed.get_value(
    &Clock::get()?,
    protocol_config.max_stale_slots,
    protocol_config.min_oracle_samples,
    true,
)?;

// 3. Interpret feed value as keccak(steam_id_64). Convention: feed returns u256 decimal.
let winner_hash: [u8; 32] = decimal_to_bytes_32(winner_id_value)?;

// 4. Match against committed identities
let winner_pubkey = if winner_hash == commitment.player_a_game_id {
    match_account.player_a
} else if winner_hash == commitment.player_b_game_id {
    match_account.player_b
} else {
    return err!(OracleWinnerNotInMatch);
};

// 5. Write into V1's proposal envelope. THIS IS THE WHOLE TRICK.
let now = Clock::get()?.unix_timestamp;
match_account.proposal_source = ProposalSource::Oracle;
match_account.proposer = ctx.accounts.relayer.key();   // whoever submitted the tx
match_account.proposed_winner = winner_pubkey;
match_account.proposed_at = now;
match_account.claim_deadline = now + tournament.dispute_window_secs as i64;

emit!(ResultProposed {                                  // V1's event — reuse
    tournament: tournament.key(),
    round, match_index,
    proposer: ctx.accounts.relayer.key(),
    proposed_winner: winner_pubkey,
    claim_deadline: match_account.claim_deadline,
    source: ProposalSource::Oracle as u8,
});
```

After this ix, the match is in V1's "pending confirmation" sub-state. The rest of the lifecycle runs through V1 instructions unchanged:
- Dispute window passes without disputes → anyone calls V1's `claim_result` (indexer cron handles automatically).
- Either player or the arbitrator disputes → V1's `dispute_result` (with V1.2's signer rule broadening — see below).
- Disputed → arbitrator calls V1's `resolve_dispute`.
- Arbitrator silent for 24h → anyone calls V1's `force_claim_disputed`.

### Modify: `instructions/dispute_result.rs` (V1's ix)

V1's signer rule says: counterparty (the non-proposer player) signs disputes. V1.2 broadens this for Oracle proposals only:

```rust
let signer = ctx.accounts.disputer.key();
let signer_in_match = signer == match_account.player_a || signer == match_account.player_b;

match match_account.proposal_source {
    ProposalSource::Player => {
        // V1 rule unchanged: counterparty only
        require!(signer_in_match && signer != match_account.proposer, NotCounterparty);
    }
    ProposalSource::Oracle => {
        // V1.2 rule: either player OR the tournament arbitrator
        let is_arbitrator = signer == tournament.arbitrator;
        require!(signer_in_match || is_arbitrator, NotAuthorized);
    }
    _ => return err!(BadProposalSource),
}
```

This is the **only** modification to V1's instructions. Everything else in V1's flow Just Works.

### Modify: `instructions/report_result.rs`

V1 already rejects `report_result` for `PlayerReported` mode. V1.2 extends this: also reject for `Oracle` mode unless `match.disputed == true`. Error: `OrganizerCannotReportInOracleMode`.

In practice this is one extra match-statement arm in V1's existing settlement-mode gate.

### Modify: `instructions/create_tournament.rs`

Validation: if `settlement_mode == Oracle`, set `tournament.arbitrator = tournament.organizer` by default (organizer is always the V1.2 arbitrator; Squads multisig reassignment is V1.3). No new params required — `arbitrator` defaults from `organizer`, and `dispute_window_secs` is already V1's.

### New ix: `instructions/set_oracle_config.rs`

Authority-gated. Writes the four `protocol_config` Switchboard fields. One-time bootstrap; idempotent. Mirrors V1.1's `set_sas_config`.

### Modify: `events.rs`

**Event-versioning convention (locked in V1.1).** Both new V1.2 events below have `event_version: u8` as their first field (value `EVENT_VERSION_V1 = 1`), per the convention established in V1.1's `Modify: events.rs` section. Do not omit when implementing.

V1.2 adds only 2 events:

```rust
#[event] pub struct MatchLobbyCommitted { tournament, round, match_index, lobby_id, committed_at }
#[event] pub struct MatchFeedBound { tournament, round, match_index, switchboard_feed }
```

**No new "oracle match reported" / "claimed" / "disputed" events.** V1's `ResultProposed`, `ResultClaimed`, `ResultDisputed`, `DisputeResolved`, and `MatchReported` events fire for oracle-path matches too — the indexer differentiates by reading `source` on `ResultProposed`. V1's `SeedRequested` / `SeedRevealed` events are reused unchanged.

### Modify: `errors.rs`

V1.2-specific errors (6, down from 14 in the original draft — VRF errors and proposal/dispute errors all come from V1):

```rust
MatchNotCommitted,
MatchAlreadyCommitted,
WrongFeedAccount,
OracleWinnerNotInMatch,
BadProposalSource,
OrganizerCannotReportInOracleMode,
NotAuthorized,  // for the broadened dispute_result signer rule
```

Errors reused from V1 unchanged: `BadSettlementMode`, `MatchNotActive`, `AlreadyProposed`, `DisputeWindowOpen`, `DisputeWindowClosed`, `NotDisputed`, `SeedNotRevealed`, `VrfCommitStale`, etc.

### Constants

```rust
pub const SWITCHBOARD_ON_DEMAND_PROGRAM_ID: Pubkey =
    pubkey!("Aio4gaXjXzJNVLtzwtNVmSqGKpANtXhybbkhtAC94ji2");      // devnet (already declared in V1's constants.rs — re-export only if needed)
```

`MAX_STALE_SLOTS` and `MIN_ORACLE_SAMPLES` are added to `ProtocolConfig` as tunable per-env values. `STALE_VRF_SLOTS` is V1's already.

### Dependencies (`Cargo.toml`)

V1 already added `switchboard-on-demand = { version = "0.12.1", features = ["anchor", "devnet"] }`. V1.2 inherits it — no new program-side dep.

### Migration

`migrate_v1_2_tournament` ix reallocs Tournament and MatchNode for the new fields (Tournament: +32 bytes for `arbitrator`; MatchNode: +129 bytes for `commitment` + `switchboard_feed`). Idempotent; anyone-callable.

Alternative: fresh program ID redeploy — recommended for devnet since V1 itself is post-V1.1 and likely the first long-lived schema anyway. Same trade-off documented in V1.1 line 195-198.

---

## SDK changes (`bracket-chain-sdk/`)

### Files to modify

1. **`src/idl/`** — regenerate via `pnpm run sync-idl`.
2. **`src/types.ts`** — add `MatchCommitment`, extend `Tournament` (`arbitrator`), extend `Match` (`commitment`, `switchboardFeed`).
3. **`src/pdas.ts`** — add `findPullFeedPda(tournament, round, matchIndex)` (wrapper over Switchboard SDK derivations). V1 already provides `findRandomnessPda` for VRF.
4. **`src/errors.ts`** — add typed errors for the 6 new program error variants.
5. **`src/methods/disputeResult.ts`** — V1's method. Update pre-flight: for `source == Oracle`, allow either player OR the arbitrator (currently only counterparty).
6. **`src/methods/reportResult.ts`** — extend V1's rejection to include `Oracle` mode (alongside `PlayerReported`).
7. **`src/match-state.ts`** — V1's effective-state helper. Extend `getMatchEffectiveState` / `getMatchActions` to surface "Oracle pending" and "Awaiting feed binding" sub-states.

### New method files

- **`src/methods/commitMatchLobby.ts`**
- **`src/methods/bindMatchFeed.ts`**
- **`src/methods/proposeResultOracle.ts`** — relayer-side; permissionless. Pre-flight: tournament is Oracle mode, match is committed + bound, feed is fresh.
- **`src/methods/setOracleConfig.ts`** — admin.
- **`src/methods/migrateV12Tournament.ts`** — idempotent migration helper.

### Extend existing `src/oracle/` (created in V1)

V1 already created `src/oracle/vrf.ts`. V1.2 adds:

- **`feedFactory.ts`** — builds the `OracleJob[]` for a Dota 2 match given `{ lobbyId, playerASteamId, playerBSteamId, matchWindowStart }`. Returns protobuf bytes ready for Switchboard's TS client. Phase 3 (CS2/Valorant) parallelizes this file.
- **`feedReader.ts`** — off-chain mirror of the on-chain `get_value` logic, used by the indexer's reconciliation path and UI freshness indicators.

### Dependencies (user-installed)

V1 already added `@switchboard-xyz/on-demand` + `@switchboard-xyz/protos`. V1.2 adds:
- `decimal.js` (for `rust_decimal::Decimal` interop in `feedReader.ts`)

---

## Indexer changes (`bracket-chain-indexer/`)

### `prisma/schema.prisma`

Extend `Tournament`:
```prisma
arbitrator             String?
```
(`vrfRandomnessAccount`, `vrfCommitSlot`, `seedRevealed` are V1's already.)

Extend `Match`:
```prisma
commitmentLobbyId        Bytes?
commitmentPlayerAGameId  Bytes?
commitmentPlayerBGameId  Bytes?
switchboardFeed          String?
```

No new tables — V1's `Proposal` and `Dispute` tables already cover oracle proposals (they carry `source: Int` in `Proposal`). One small addition: a `MatchFeed` table for feed freshness UI:

```prisma
model MatchFeed {
  tournamentAddress  String
  round              Int
  matchIndex         Int
  switchboardFeed    String
  jobsHash           String
  createdAt          DateTime
  lastUpdatedSlot    BigInt?
  lastValue          Bytes?
  @@id([tournamentAddress, round, matchIndex])
  @@index([switchboardFeed])
}
```

### Extend existing `src/oracle/` module (created in V1)

V1 already provides `src/oracle/auto-claim-cron.service.ts` and `src/oracle/vrf-reveal-cron.service.ts`. V1.2 adds:

- **`feed-factory.service.ts`** — wraps SDK's `feedFactory.ts`. Reuses V1's `SWITCHBOARD_PAYER_KEYPAIR` env (already funded for VRF); creates feeds on behalf of organizers (indexer eats incremental SOL cost on devnet; V1.3 introduces per-organizer payment).
- **`feed-staleness-monitor.service.ts`** — for active oracle matches, polls bound feeds and logs warnings when `lastUpdatedSlot` falls behind `max_stale_slots`.
- **`oracle.controller.ts`** — endpoints:
  - `POST /feeds/match/:tournament/:round/:matchIndex` → indexer creates feed, returns pubkey for SDK to pass to `bind_match_feed`.
  - `GET /feeds/match/:tournament/:round/:matchIndex` → freshness, last value, last update slot.

V1's existing notifications module already pushes `ResultProposed` to the counterparty — for `source == Oracle`, push targets are both players + the arbitrator instead. One small extension in the existing service, no new module.

V1's existing `auto-claim-cron.service.ts` already handles permissionless `claim_result` for any proposal past its deadline — including oracle proposals. **Nothing to add in the cron.** V1's `vrf-reveal-cron.service.ts` handles VRF reveal for Oracle-mode tournaments unchanged.

### `src/webhooks/helius-parser.service.ts`

V1 already handles `ResultProposed`, `ResultConfirmed`, `ResultDisputed`, `ResultClaimed`, `DisputeResolved`, `MatchReported`, `SeedRequested`, `SeedRevealed`. V1.2 adds handlers for 2 new events:
- `MatchLobbyCommitted` → upsert Match row's commitment fields.
- `MatchFeedBound` → upsert Match row's `switchboardFeed` + insert `MatchFeed` row.

### Environment vars

V1 already provides `SWITCHBOARD_PAYER_KEYPAIR`, `SWITCHBOARD_QUEUE`. V1.2 adds:
- `STEAM_API_KEY` — held transiently when creating a feed, passed via Switchboard `variableOverrides` (sealed to TEE), then discarded from memory.
- `OPENDOTA_API_KEY` — optional; free tier sufficient for low volume.

### Dependencies (user-installed)

V1 already added `@switchboard-xyz/on-demand`. V1.2 adds:
- `@switchboard-xyz/protos` (if not already present from V1's protos use for VRF payloads).

---

## Frontend changes (`BracketChain-Frontend/`)

The V1 plan landed the action-dispatcher pattern in `ReportResultModal.tsx` (`getMatchActions(match, tournament, viewer, now)` dispatches to `ProposeResultPanel` / `ConfirmOrDisputePanel` / `ClaimResultPanel` / `ResolveDisputePanel` / `ForceClaimPanel`). V1.2 adds two new panel types and extends the dispatcher.

### Files to modify

1. **`types/tournament.ts`** — add `MatchCommitment`; extend `Tournament` with `arbitrator`; extend `Match` with `commitment`, `switchboardFeed`.
2. **`lib/indexerToTournamentState.ts`** — map new fields.
3. **`features/tournament/create/CreateTournament.tsx`** — when `settlementMode == Oracle` is picked, surface `arbitratorAddress` field (defaults to organizer wallet).
4. **`features/tournament/steps/DetailsStep.tsx`** — extend with arbitrator picker (defaults to organizer; will accept Squads multisig in V1.3).
5. **`features/tournament/view/BracketView.tsx`** — match tooltip: add `OraclePendingBadge` for matches with `source == Oracle` in the dispute window. Reuse V1's `DisputeCountdown`.
6. **`features/tournament/view/ReportResultModal.tsx`** — dispatcher gains two new branches:
   - For Oracle-mode match with no commitment yet (organizer-viewing): `CommitAndBindPanel` (combined modal that runs commit_match_lobby → indexer creates feed → bind_match_feed).
   - For Oracle-mode match awaiting oracle (`commitment.is_some() && proposal_source == None`): "Awaiting oracle. Last feed update Xs ago." Read-only.
   - All other Oracle-mode states (`PendingConfirmation`, `Disputed`, etc.) reuse V1's existing panels.

### New files

1. **`features/tournament/view/CommitAndBindPanel.tsx`** — multi-step organizer modal: generates random `lobby_id`, displays it for copy-paste into Dota 2 lobby name, calls `commitMatchLobby`, polls indexer for feed creation, calls `bindMatchFeed`. Toast on each step.
2. **`features/tournament/view/OraclePendingPanel.tsx`** — read-only; shows feed freshness, last update slot, expected proposal arrival window.
3. **`hooks/useMatchFeed.ts`** — TanStack Query against `GET /feeds/match/...`; staleTime 15s during active matches.
4. **`components/OraclePendingBadge.tsx`** — small badge for the match tooltip.
5. **`components/SettlementBadge.tsx`** — extend V1's `MatchStateBadge` or add as a sibling: "OrganizerOnly | PlayerReported | Oracle" tournament-level badge.

V1 already provides: `MatchStateBadge`, `DisputeCountdown`, `DisputeReasonModal`, `useMatchEffectiveState`, `useViewerMatchActions`, `useVrfStatus`, all proposal/dispute panels, and the `TournamentSidebar` VRF gating on the Start button. **None of them need V1.2-specific changes** — they're already source-agnostic, and VRF UX is shared across PlayerReported and Oracle modes.

### Out of frontend scope for V1.2

- Arbitrator-recruitment / multisig-onboarding UX (V1.3).
- Per-feed cost UI showing organizer how much SOL the indexer spent (V1.3).
- Dota 2 replay viewer / OpenDota deep links from match tooltip (nice-to-have, not blocking).

---

## Switchboard bootstrap (one-time, devnet)

After program redeploy with V1.2 program ID:

1. **Verify V1's Switchboard payer is funded** for the incremental feed-creation cost (top up to ~5 SOL on devnet if needed — V1 budgeted for VRF reveals; feed creation adds ~0.01 SOL each).
2. **Call `initialize_protocol`** to bootstrap the new protocol config PDA.
3. **Call `set_sas_config`** with V1.1's credential + schemas (or re-bootstrap if SAS redeployed too).
4. **Call `set_oracle_config`** with `{ max_stale_slots: 100, min_oracle_samples: 5 }`. (Switchboard program ID + queue are already set by V1's protocol config bootstrap.)
5. **Smoke test** before announcing V1.2 to organizers.

Document in `bracketchain-main/README.md` under "V1.2 setup."

---

## Verification (end-to-end devnet smoke)

The verification matrix is significantly shorter than the original draft because V1's tests already cover proposal/confirm/dispute/claim/resolve mechanics. V1.2 only tests the **new surface**:

1. **Commitment tests**:
   - `commit_match_lobby` writes correct `player_*_game_id` sourced from Participant rows.
   - Re-commit → `MatchAlreadyCommitted`.
   - `commit_match_lobby` for `OrganizerOnly`/`PlayerReported` tournament → `BadSettlementMode`.
2. **Feed binding tests**:
   - `bind_match_feed` rejects non-Switchboard-owned account → `WrongFeedAccount`.
   - `bind_match_feed` before `commit_match_lobby` → `MatchNotCommitted`.
3. **Oracle proposal tests** (with mocked PullFeed account via test util):
   - Happy path: `propose_result_oracle` writes V1's proposal envelope with `source = Oracle`, `proposed_winner` matching one of the committed game IDs.
   - Wrong feed pubkey: → `WrongFeedAccount`.
   - Stale feed (older than `max_stale_slots`): `get_value` error propagates.
   - Feed value not matching either committed game ID: → `OracleWinnerNotInMatch`.
   - Re-proposing after a proposal exists: → `AlreadyProposed`.
4. **Dispute signer broadening tests**:
   - Oracle proposal disputed by player_a: success.
   - Oracle proposal disputed by player_b: success.
   - Oracle proposal disputed by arbitrator (= organizer in V1.2 default): success.
   - Oracle proposal disputed by an unrelated wallet: → `NotAuthorized`.
5. **End-to-end happy path**: create Oracle-mode Dota 2 tournament → V1's VRF reveal → V1's `start_tournament` → commit + bind for match 0 → mocked feed returns player_a's hash → `propose_result_oracle` → dispute window passes → V1's indexer cron calls `claim_result` → bracket advances. Verify all V1 events emit and `Proposal.source == Oracle` in indexer.
6. **End-to-end dispute path**: same setup → after `propose_result_oracle`, arbitrator calls V1's `dispute_result` with reason code → match flagged `disputed` → arbitrator calls V1's `resolve_dispute` with player_b → match finalizes with player_b.
7. **24h organizer-silence path**: oracle proposal → dispute → arbitrator silent → V1's `force_claim_disputed` accepts oracle proposal after 24h. (Inherits V1's cron.)
8. **MCP check**: re-run `Solana_Expert__Ask_For_Help` with the final `PullFeedAccountData::parse` / `get_value` snippet to sanity-check the parameters against latest Switchboard guidance.

VRF lifecycle tests live in V1's suite and apply transitively — no V1.2 work to revalidate VRF.

---

## Open questions to resolve before kickoff

1. **OracleJob secret strategy.** `STEAM_API_KEY` is sealed to the TEE via Switchboard `variableOverrides` per-feed. Rotating the key invalidates every active feed. Mitigation: match-scoped feeds (one per match, destroyed after claim) keep the blast radius small. Worth confirming with a Switchboard SAIL engineer that this is the recommended pattern.
2. **Feed reuse across rounds.** Could theoretically reuse one feed across all matches via re-binding `variableOverrides`. Rejected: per-match feeds make the `match.switchboard_feed == bound_feed` invariant trivially verifiable. Revisit if creation cost becomes a problem.
3. **Lobby ID UX.** Recommend frontend-generated (random 16 bytes), displayed with copy button. Organizer pastes into the Dota 2 lobby name field. This is the human-side ceremony linking on-chain commitment to actual lobby.
4. **Dota 2 Steam Web API stability.** Does `GetMatchHistoryBySequenceNum` reliably return the right match for "two Steam IDs played together after slot X"? Test against ~10 real Dota 2 matches before locking the OracleJob shape.
5. **Feed cost passthrough.** V1.2 indexer eats SOL cost (sub-cent on devnet). Defer to V1.3.

---

## Explicitly out of scope (V1.3+)

- **CS2 / Valorant / LoL adapters.** Each needs its own `OracleJob` template, API key, and identity mapping. V1.1's `SupportedGame` enum reserves the variants.
- **BR placement payouts** (Apex, PUBG, Fortnite). Different `MatchOutcome` envelope. Phase 4+.
- **Squads multisig as arbitrator.** V1.2 hardcodes `arbitrator = organizer`. Multisig is a small `set_arbitrator(Pubkey)` ix + UX.
- **Per-organizer feed cost passthrough** (V1.3).
- **DIA as secondary data source.** Steam + OpenDota is sufficient for Dota 2.
- **Slashing for oracle misreports.** Switchboard has crank-level slashing already.
- **Cross-tournament arbitrator reputation.**
- **Webhook-based feed activation** (push). Switchboard On-Demand is pull-only by design — architectural non-goal.

---

## Critical files (quick reference)

**Program:**
- `bracket-chain-programs/programs/bracket-chain/src/lib.rs` — add 5 new ix entrypoints (`commit_match_lobby`, `bind_match_feed`, `propose_result_oracle`, `set_oracle_config`, `migrate_v1_2_tournament`); extend `create_tournament`, `report_result`, `dispute_result`
- `bracket-chain-programs/programs/bracket-chain/src/state/tournament.rs` — add `arbitrator` (VRF fields are V1's)
- `bracket-chain-programs/programs/bracket-chain/src/state/match_node.rs` — add `commitment`, `switchboard_feed`
- `bracket-chain-programs/programs/bracket-chain/src/state/match_commitment.rs` — new
- `bracket-chain-programs/programs/bracket-chain/src/state/protocol_config.rs` — add Switchboard config fields (`max_stale_slots`, `min_oracle_samples`; Switchboard program ID + queue are V1's)
- `bracket-chain-programs/programs/bracket-chain/src/instructions/commit_match_lobby.rs`, `bind_match_feed.rs`, `propose_result_oracle.rs`, `set_oracle_config.rs`, `migrate_v1_2_tournament.rs` — new
- `bracket-chain-programs/programs/bracket-chain/src/instructions/dispute_result.rs` — broaden signer rule for `source == Oracle`
- `bracket-chain-programs/programs/bracket-chain/src/instructions/report_result.rs` — reject for `Oracle` mode unless `disputed`
- `bracket-chain-programs/programs/bracket-chain/src/instructions/create_tournament.rs` — default `arbitrator = organizer` for Oracle mode
- `bracket-chain-programs/programs/bracket-chain/src/errors.rs` — add 6 V1.2-specific variants (VRF errors are V1's)
- `bracket-chain-programs/programs/bracket-chain/src/events.rs` — add 2 events (`MatchLobbyCommitted`, `MatchFeedBound`)
- `bracket-chain-programs/programs/bracket-chain/src/constants.rs` — no new constants (`STALE_VRF_SLOTS`, `SWITCHBOARD_ON_DEMAND_PROGRAM_ID` are V1's)

**SDK:**
- `bracket-chain-sdk/src/types.ts`, `pdas.ts`, `errors.ts`, `index.ts` — extensions
- `bracket-chain-sdk/src/methods/` — 5 new method files + edits to `disputeResult.ts`, `reportResult.ts`
- `bracket-chain-sdk/src/oracle/feedFactory.ts`, `feedReader.ts` — new (`vrf.ts` is V1's)
- `bracket-chain-sdk/src/match-state.ts` — extend V1's helper with Oracle sub-states

**Indexer:**
- `bracket-chain-indexer/src/oracle/` — extend V1's module with `feed-factory.service.ts`, `feed-staleness-monitor.service.ts`, `oracle.controller.ts` (V1 already provides `auto-claim-cron.service.ts`, `vrf-reveal-cron.service.ts`)
- `bracket-chain-indexer/src/webhooks/helius-parser.service.ts` — handlers for 2 new events; V1's `ResultProposed`/`ResultDisputed`/`SeedRequested`/`SeedRevealed`/etc handlers reused unchanged
- `bracket-chain-indexer/prisma/schema.prisma` — extend `Tournament` (`arbitrator`) + `Match` (commitment + feed fields); new `MatchFeed` table
- `bracket-chain-indexer/src/notifications/notifications.service.ts` — extend push-target logic for `source == Oracle`

**Frontend:**
- `BracketChain-Frontend/types/tournament.ts` — extend
- `BracketChain-Frontend/lib/indexerToTournamentState.ts` — map new fields
- `BracketChain-Frontend/features/tournament/create/CreateTournament.tsx`, `steps/DetailsStep.tsx` — arbitrator picker
- `BracketChain-Frontend/features/tournament/view/BracketView.tsx` — Oracle pending badge
- `BracketChain-Frontend/features/tournament/view/ReportResultModal.tsx` — extend V1's dispatcher with two Oracle branches
- `BracketChain-Frontend/features/tournament/view/CommitAndBindPanel.tsx`, `OraclePendingPanel.tsx` — new
- `BracketChain-Frontend/hooks/useMatchFeed.ts` — new (`useVrfStatus.ts` is V1's; `TournamentSidebar` VRF gating is V1's)
- `BracketChain-Frontend/components/OraclePendingBadge.tsx`, `SettlementBadge.tsx` — new

**Docs:**
- `bracketchain-main/README.md` — V1.2 setup + V1 → V1.2 delta.
- `bracketchain-v1.1-plan.md` — note that the `Manual` → `OrganizerOnly` rename happens in V1 (not V1.2).
- `bracketchain-v1-player-reported-plan.md` — note that `ProposalSource::Oracle` is activated in V1.2.
