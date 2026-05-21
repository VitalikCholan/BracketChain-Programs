# BracketChain V1 — Player-Reported Results with Dispute Window

## Context

MVP ships with **organizer-signed** match results: `report_result` requires `signer == tournament.organizer`. This is fine for solo-organizer demo tournaments but doesn't scale — for a 64-player Dota 2 tournament with 32+ matches a day, the organizer becomes a bottleneck and a single point of trust.

V1 introduces the **player-reported flow**: either player submits the result, the counterparty confirms or disputes within a window, the organizer adjudicates disputes, and unresponded proposals auto-finalize after the window. This is the dominant pattern for community-run esports tournaments (FACEIT, ESEA, GamersClub all work this way).

**Strategic gate:** this plan is the architectural prerequisite for V1.2 (oracle settlement). The proposal/dispute primitive built here is what V1.2 reuses — the oracle is just another "proposer." If V1.2 ships first, V1 will require a second parallel dispute system and a `MatchNode` migration. Sequencing this first turns V1.2 into a thin layer instead of a parallel implementation.

**Sequencing constraint (locked) — VRF ownership.** This plan **owns Switchboard VRF** for bracket seeding (`request_seed` / `reveal_seed`, plus `tournament.seed_hash` and `seed_revealed`). VRF lives here, not in V1.2, because real money enters the system at `PlayerReported` mode (not at `Oracle` mode) — the moment entry fees are non-trivial, validator-manipulable seeding becomes an attack surface that the organizer can exploit. The following downstream plans **depend on VRF shipping here**:

- **V1.2 (Oracle settlement)** — Oracle-mode tournaments inherit `settlement_mode != OrganizerOnly` gating, so VRF is auto-required. V1.2 adds zero VRF surface.
- **V1 formats plan, Phase C (Swiss)** — uses `tournament.seed_hash` for round-1 pairing entropy.
- **V2-C (GameServer attestation)** — same `settlement_mode != OrganizerOnly` gate inherits VRF.

**Do not extract VRF from this plan.** If V1.2 ships before this plan (e.g., because Oracle features look business-critical), VRF either has to be duplicated in V1.2 (which then has to be removed when this plan lands — wire-breaking change) or moved to V1.2 (which forces formats Phase C to wait for V1.2 redeploy). Either path is more expensive than respecting this sequencing.

**Scope:**
- Player-reported results (`propose_result`).
- Counterparty confirmation (`confirm_result`).
- Counterparty dispute (`dispute_result`).
- Permissionless timeout claim (`claim_result`).
- Organizer adjudication of disputes (`resolve_dispute`).
- Configurable per-tournament dispute window.
- Notification surface for "your match needs confirmation."
- **Switchboard VRF for bracket seeding** (`request_seed` / `reveal_seed`). Required for any tournament where `settlement_mode != OrganizerOnly` — the moment real money is at stake, validator-manipulable seeding (e.g., blockhash-derived) becomes an attack surface. VRF is opt-out for `OrganizerOnly` (existing implicit seeding retained for trust-mode tournaments).

**Out of scope** (reserved for later plans):
- Oracle-based proposal (V1.2 — but the `ProposalSource::Oracle` enum variant is reserved here).
- Game-server attestation (V2).
- Staked-arbiter dispute resolution (V3).
- Cross-tournament arbiter reputation.

---

## The proposal/dispute primitive (architectural core)

The single most important design choice in this plan: **the proposal envelope is generic over the proposer**. Whether a result is proposed by a player wallet (V1), an oracle relayer reading a Switchboard feed (V1.2), or a game server signing with a registered key (V2), the on-chain shape is the same. Only the validation logic for *who can propose* differs.

### Generic envelope on `MatchNode`

```rust
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, InitSpace, PartialEq, Eq)]
pub enum ProposalSource {
    None,         // 0 — no proposal yet (default)
    Player,       // 1 — player-reported (V1)
    Oracle,       // 2 — reserved for V1.2; rejected in V1
    GameServer,   // 3 — reserved for V2
}

// Added to MatchNode:
pub proposal_source: ProposalSource,
pub proposer: Pubkey,                 // wallet that submitted the proposal
pub proposed_winner: Pubkey,
pub proposed_at: i64,
pub claim_deadline: i64,              // proposed_at + tournament.dispute_window_secs
pub disputed: bool,
pub dispute_reason: u8,               // 0 = none; populated only when disputed
```

Account-size cost: +1 +32 +32 +8 +8 +1 +1 = 83 bytes per match. For a 64-player single-elim bracket (~32 matches), that's ~2.6KB total — fine.

### Sub-states are derived, not stored

`MatchStatus` stays as `Pending | Active | Completed`. The proposal sub-states are computed from data:

| Effective state            | Derivation                                                        |
|----------------------------|-------------------------------------------------------------------|
| Awaiting result            | `status == Active && proposal_source == None`                     |
| Pending confirmation       | `status == Active && proposal_source != None && !disputed && now < claim_deadline` |
| Past deadline (claimable)  | `status == Active && proposal_source != None && !disputed && now >= claim_deadline` |
| Disputed                   | `status == Active && disputed == true`                            |
| Finalized                  | `status == Completed`                                             |

`★ Insight ─────────────────────────────────────`
Keeping `MatchStatus` small is a deliberate forward-compat choice. If we baked sub-states into the enum (`PendingConfirmation`, `Disputed`), V1.2 would either need a new variant for oracle proposals or would have to reuse `PendingConfirmation` confusingly. Deriving sub-states from data lets V1.2 add `ProposalSource::Oracle` without touching the enum — the same forward-compat-enum trick V1.1 used for `SupportedGame`.
`─────────────────────────────────────────────────`

### Per-tournament window configuration

`Tournament.dispute_window_secs: u32` (already proposed in V1.2; introduced here in V1). Default 3600 (1 hour). Bounds: `[0, 86_400]`. Set at `create_tournament` time, immutable after.

---

## Program changes (`bracket-chain-programs/`)

### Modify: `state/tournament.rs`

Add (after the V1.1 `settlement_mode` field, before `bump`):

```rust
pub dispute_window_secs: u32,            // proposal-to-finalize window
pub settlement_mode: SettlementMode,     // already from V1.1 — repurposed here
pub vrf_randomness_account: Pubkey,      // Switchboard RandomnessAccountData PDA; default zero for OrganizerOnly
pub vrf_commit_slot: u64,                // staleness / replay protection
pub seed_revealed: bool,                 // false until reveal_seed succeeds; ignored for OrganizerOnly
```

V1.1's `SettlementMode` enum gets one extension: rename `Manual` → `OrganizerOnly`, and add `PlayerReported`. The full enum after V1:

```rust
pub enum SettlementMode {
    OrganizerOnly,    // 0 — only organizer signs report_result (MVP behavior)
    PlayerReported,   // 1 — V1: propose/confirm/dispute flow
    Oracle,           // 2 — reserved for V1.2
    // Discriminator 3 intentionally free — V2 fills it with GameServer.
    // `Hybrid` was dropped in V1.1's cleanup pass; see that plan's Scope decisions.
}
```

**Naming migration**: V1.1 ships with the variant named `Manual`. The rename to `OrganizerOnly` is a breaking change to the IDL but no on-chain data migration is needed (the discriminator value `0` stays the same). SDKs need to re-export. The rename is purely for human readability now that there are multiple non-oracle modes.

Account-size cost: +4 (`dispute_window_secs`) +32 (`vrf_randomness_account`) +8 (`vrf_commit_slot`) +1 (`seed_revealed`) = 45 bytes per Tournament.

### Modify: `state/match_node.rs`

Add the proposal envelope fields listed in the "Generic envelope" section above. Per the V1.2 plan, this is the migration point — V1.2 layers oracle-specific fields (`switchboard_feed`, `oracle_winner`, etc.) on top, but the proposal envelope itself is V1's deliverable.

**Also cherry-pick PDA seed schema-prep from formats plan Phase A.** Since this redeploy already changes MatchNode's struct layout, fold in the seed-shape change at the same time — paying the breaking-change cost once instead of twice.

```rust
// Add to MatchNode struct:
pub bracket: u8,         // 0 for SingleElim/RR/Swiss; 0/1/2 for DE (when formats Phase B ships)
pub score_a: u16,        // 0 if unreported
pub score_b: u16,        // 0 if unreported
```

**New PDA seed for MatchNode (replaces MVP seed):**

```rust
&[
    b"match",
    tournament.key().as_ref(),
    &[bracket],                              // NEW byte
    &[round],
    &match_index.to_le_bytes(),
]
```

**Add `state/format.rs`:**

```rust
pub enum TournamentFormat {
    SingleElim,    // 0 — Phase 1 default; only enabled variant
    DoubleElim,    // 1 — reserved; create_tournament rejects until formats Phase B
    Swiss,         // 2 — reserved; create_tournament rejects until formats Phase C
    RoundRobin,    // 3 — reserved; create_tournament rejects until formats Phase A
}
```

**Add `format: TournamentFormat` to Tournament struct** (+1 byte). `create_tournament` accepts `format` param and rejects non-`SingleElim` with `FormatNotYetSupported` error until formats plan lifts the gate.

**Extend `report_result` signature with `score_a: u16, score_b: u16`** params. Default zero is backward-compatible. Validation: if both non-zero, `winner` must match the higher score; otherwise `ScoreInconsistentWithWinner`.

**Why ship this here, not in formats plan Phase A:**

- The PDA seed shape (`bracket: u8` byte) is a **breaking change** to all MatchNode addresses. Shipping it standalone in Phase 3 forces a second program redeploy with a new program ID.
- This plan (V1 player-reported) is already breaking MatchNode struct via the proposal envelope — fresh program ID redeploy is already required for this Phase 1 bundle.
- Combining both schema changes into one redeploy means **formats Phase A becomes purely additive** (lift the gate, add `init_round_robin_chunk` dispatch, add `finalize_round_robin` ix) — no schema or PDA changes needed at Phase 3 time.
- Cost in this redeploy: ~5 bytes per MatchNode, +1 byte per Tournament, ~50 lines of Rust. Cost of NOT doing this: an entire second redeploy with all the operational tax (new program ID, SDK republish, indexer migration, frontend env-var update, third-party integration breakage).

This is **schema-prep, not feature-ship.** Round Robin / Double Elim / Swiss logic stays out of scope here; only the data shape they need gets pre-shipped.

### Modify: `state/protocol_config.rs`

Add:
```rust
pub default_dispute_window_secs: u32,    // 3600
pub max_dispute_window_secs: u32,        // 86_400 (24h ceiling)
pub switchboard_program: Pubkey,         // Switchboard On-Demand program ID (devnet: Aio4...4ji2)
pub switchboard_queue: Pubkey,           // shared devnet queue PDA
```
Set via the existing `set_protocol_config` admin ix.

### New ix: `instructions/request_seed.rs`

Organizer-signed. Two-phase commit/reveal pattern from Switchboard's randomness tutorial — required because VRF reveal must happen in a **later** slot than the commit to be unmanipulable.

Handler:
- Tournament status must be `Registration`. Move to `PendingBracketInit`.
- Tournament must not already have a committed-but-unrevealed VRF (`vrf_commit_slot == 0 || seed_revealed`).
- Allocate Switchboard `RandomnessAccountData` (CPI to Switchboard On-Demand).
- Set `tournament.vrf_randomness_account = randomness_pda`, `tournament.vrf_commit_slot = Clock::get()?.slot`.
- Emit `SeedRequested { tournament, randomness_pda, commit_slot }`.

Critical rule from the Switchboard randomness tutorial: **charge VRF fee at commit, not at reveal.** Otherwise an organizer who dislikes the revealed seed could refuse to reveal and re-roll. Here, the entry-fee economics already incentivize reveal (organizer's `organizer_deposit` is locked from `create_tournament`), so this is enforced indirectly — but the fee for the Switchboard request itself is paid in `request_seed`.

### New ix: `instructions/reveal_seed.rs`

**Permissionless** — anyone can call after the commit slot. This is the load-bearing property that makes the organizer-refuses-to-reveal attack impossible: even if the organizer ghosts, the indexer cron (or any spectator) reveals the seed and the tournament proceeds.

Handler:
- Tournament status must be `PendingBracketInit` and `seed_revealed == false`.
- Reject if `Clock::get()?.slot > tournament.vrf_commit_slot + STALE_VRF_SLOTS` → `VrfCommitStale`. The organizer must re-request.
- Reject if `Clock::get()?.slot <= tournament.vrf_commit_slot` (reveal in same slot as commit is manipulable) → `VrfNotRevealable`.
- Read the `RandomnessAccountData`, write `tournament.seed_hash = randomness.get_value(&clock)?` (returns `[u8; 32]`).
- Set `tournament.seed_revealed = true`.
- Emit `SeedRevealed { tournament, seed_hash, slot }`.

### Modify: `instructions/start_tournament.rs`

Two changes:
1. If `tournament.settlement_mode != OrganizerOnly`, require `tournament.seed_revealed == true` → `SeedNotRevealed`. `OrganizerOnly` tournaments keep the existing implicit/join-order seeding (cheaper, no VRF round-trip, fine for trust-mode).
2. For VRF-seeded tournaments, derive bracket assignments deterministically from `tournament.seed_hash` via Fisher-Yates (client-side via SDK; program validates the `MatchInitDescriptor[]` is consistent with the seed).

`★ Insight ─────────────────────────────────────`
Deterministic-from-seed bracket generation client-side keeps `start_tournament` cheap (the program only validates rather than computes the shuffle). This matters because Fisher-Yates over 128 players with on-chain RNG would push the instruction's compute units into expensive territory. The seed is the commitment; the descriptor list is the proof-of-work. Anyone can re-derive the same brackets from the same seed off-chain.
`─────────────────────────────────────────────────`

### New ix: `instructions/propose_result.rs`

Player-signed. Accounts: `proposer (signer)`, `tournament`, `match_account`, `participant_proposer` (PDA for proposer's participation record). Inputs: `round`, `match_index`, `proposed_winner: Pubkey`.

Handler:

```rust
require!(tournament.settlement_mode == SettlementMode::PlayerReported, BadSettlementMode);
require!(tournament.status == TournamentStatus::Active, NotActive);
require!(match_account.status == MatchStatus::Active, MatchNotActive);
require!(match_account.proposal_source == ProposalSource::None, AlreadyProposed);

// Proposer must be one of the two players
let proposer = ctx.accounts.proposer.key();
require!(
    proposer == match_account.player_a || proposer == match_account.player_b,
    ProposerNotInMatch,
);

// Proposed winner must also be in the match
require!(
    proposed_winner == match_account.player_a || proposed_winner == match_account.player_b,
    WinnerNotInMatch,
);

let now = Clock::get()?.unix_timestamp;
match_account.proposal_source = ProposalSource::Player;
match_account.proposer = proposer;
match_account.proposed_winner = proposed_winner;
match_account.proposed_at = now;
match_account.claim_deadline = now + tournament.dispute_window_secs as i64;

emit!(ResultProposed {
    tournament: tournament.key(),
    round, match_index,
    proposer, proposed_winner,
    claim_deadline: match_account.claim_deadline,
    source: ProposalSource::Player as u8,
});
```

Note: nothing finalizes here. The match stays `Active`; advancement happens in `confirm_result` or `claim_result`.

### New ix: `instructions/confirm_result.rs`

Counterparty-signed. Accounts: same shape as `propose_result` plus the `next_match` / placements for advancement (mirrors `report_result`). Inputs: `round`, `match_index`.

Handler:
- Require `match.proposal_source != None && !match.disputed`.
- Require signer is the *other* player (not the proposer).
- Set `match.winner = match.proposed_winner`, `match.status = Completed`.
- Branch on final vs non-final: `advance_winner()` or `distribute_prizes()` — identical to `report_result`'s tail.
- Emit `MatchReported` (preserves existing indexer parser) + `ResultConfirmed` (new, for provenance).

### New ix: `instructions/dispute_result.rs`

Counterparty-signed. Inputs: `round`, `match_index`, `reason_code: u8`.

Handler:
- Require `match.proposal_source != None && !match.disputed`.
- Require `now < match.claim_deadline` (dispute window must be open).
- Require signer is the *other* player.
- Set `match.disputed = true`, `match.dispute_reason = reason_code`.
- Match remains `Active`; only `resolve_dispute` (organizer) or, if organizer is unresponsive past 24h, a permissionless re-propose path can move it forward.
- Emit `ResultDisputed`.

`reason_code` enum (purely informational on-chain; full reason text lives in indexer's `Dispute` table):
- `0` Wrong winner reported
- `1` Match wasn't played
- `2` Replay/proof shows different outcome
- `3` Cheating alleged
- `4` Other

### New ix: `instructions/claim_result.rs`

**Permissionless** — anyone can call after the deadline. Inputs: `round`, `match_index`, plus advancement accounts.

Handler:
- Require `match.proposal_source != None && !match.disputed`.
- Require `now >= match.claim_deadline`.
- Mirror `confirm_result`'s finalization path (set winner, advance, emit `MatchReported` + `ResultClaimed`).

The permissionless property matters: without it, an unresponsive counterparty could indefinitely stall an unconfirmed-but-undisputed match, and the proposer (who's *not* the counterparty) couldn't move it forward without organizer help. This ix lets *anyone* (including bots, the indexer's cron, the proposer themselves, or a spectator) close the window.

### New ix: `instructions/resolve_dispute.rs`

Organizer-signed. Inputs: `round`, `match_index`, `winner: Pubkey`, `placements: Vec<Pubkey>` (only required for final).

Handler:
- Require `match.disputed == true`.
- Require signer is `tournament.organizer`.
- Require `winner == match.player_a || winner == match.player_b`.
- Override: `match.winner = winner`, `match.disputed = false` (history kept via events), `match.status = Completed`.
- Advance or distribute as usual.
- Emit `DisputeResolved { resolver: organizer, winner } ` + `MatchReported`.

### New ix: `instructions/force_claim_disputed.rs` (optional, recommended)

Permissionless fallback for organizer-unresponsive scenarios. Inputs: `round`, `match_index`.

Handler:
- Require `match.disputed == true`.
- Require `now >= match.proposed_at + ORGANIZER_RESPONSE_WINDOW_SECS` (default 24h, configurable on `ProtocolConfig`).
- Accept the original proposal (`match.winner = match.proposed_winner`).
- Emit `DisputeAutoResolved { source: TimedOut, winner: proposed_winner }`.

This is the safety valve for "organizer ghosted the tournament after a dispute was raised." Per the user's V1 spec: *"⚠️ Error: Organizer unresponsive >24h → V1: default to proposal."*

`★ Implementation note ─────────────────────────`
This ix is the failure-mode contingency, not the normal path. The 24h window is intentionally long — short enough that prize pools eventually release, long enough that a real organizer with a real conflict (e.g., reviewing demos) has time. Surface a countdown in the indexer + frontend so disputers know exactly when their option to force-claim opens.
`─────────────────────────────────────────────────`

### Modify: `instructions/report_result.rs`

Behavior depends on `tournament.settlement_mode`:
- `OrganizerOnly`: unchanged (existing MVP behavior).
- `PlayerReported`: **rejected** with `OrganizerCannotReportInPlayerMode`. Forces the organizer to use `resolve_dispute` (which requires a dispute first).
- `Oracle`: V1.2 handles this (rejected here pre-V1.2 with `BadSettlementMode`).

This is the load-bearing safety check — without it, an organizer in a player-reported tournament could front-run player proposals.

### Modify: `instructions/create_tournament.rs`

Extend signature with `dispute_window_secs: u32` and the existing V1.1 `settlement_mode: SettlementMode`. Validation:
- `dispute_window_secs` must be in `[0, protocol_config.max_dispute_window_secs]`. Zero is allowed (organizer-trust mode where any proposal is instantly final — useful for friend-group tournaments).
- If `settlement_mode == OrganizerOnly`, `dispute_window_secs` is ignored (set to 0).

Update `TournamentCreated` event to include both fields.

### Modify: `events.rs`

**Event-versioning convention (locked in V1.1).** Every `#[event]` struct in the program — both the 7 MVP events and the 7 new events added below — has `event_version: u8` as its **first field**, with initial value `EVENT_VERSION_V1 = 1`. See V1.1 plan's `Modify: events.rs` section for the rationale and the indexer parser pattern. The structs below show this field first; do not omit when implementing.

Add:

```rust
#[event] pub struct ResultProposed {
    pub event_version: u8,                // = EVENT_VERSION_V1; first field per V1.1 convention
    pub tournament: Pubkey,
    pub round: u8,
    pub match_index: u16,
    pub proposer: Pubkey,
    pub proposed_winner: Pubkey,
    pub claim_deadline: i64,
    pub source: u8,                // ProposalSource as u8
}

#[event] pub struct ResultConfirmed {
    pub event_version: u8,
    pub tournament: Pubkey,
    pub round: u8,
    pub match_index: u16,
    pub confirmer: Pubkey,
}

#[event] pub struct ResultDisputed {
    pub event_version: u8,
    pub tournament: Pubkey,
    pub round: u8,
    pub match_index: u16,
    pub disputer: Pubkey,
    pub reason_code: u8,
}

#[event] pub struct ResultClaimed {
    pub event_version: u8,
    pub tournament: Pubkey,
    pub round: u8,
    pub match_index: u16,
    pub claimer: Pubkey,           // whoever called the permissionless ix
}

#[event] pub struct DisputeResolved {
    pub event_version: u8,
    pub tournament: Pubkey,
    pub round: u8,
    pub match_index: u16,
    pub resolver: Pubkey,
    pub final_winner: Pubkey,
    pub source: u8,                // 0=OrganizerManual, 1=TimedOut
}

#[event] pub struct SeedRequested {
    pub event_version: u8,
    pub tournament: Pubkey,
    pub randomness_pda: Pubkey,
    pub commit_slot: u64,
}

#[event] pub struct SeedRevealed {
    pub event_version: u8,
    pub tournament: Pubkey,
    pub seed_hash: [u8; 32],
    pub slot: u64,
}
```

The existing `MatchReported` event is still emitted from `confirm_result`, `claim_result`, `resolve_dispute`, and `force_claim_disputed`. **This is the contract the indexer's lean parser depends on** — every finalization path emits `MatchReported`, no exceptions.

### Modify: `errors.rs`

Add: `AlreadyProposed`, `ProposerNotInMatch`, `WinnerNotInMatch`, `NotCounterparty`, `BadSettlementMode`, `MatchNotActive`, `DisputeWindowOpen`, `DisputeWindowClosed`, `NotDisputed`, `OrganizerCannotReportInPlayerMode`, `OrganizerResponseWindowOpen`, `DisputeWindowOutOfRange`, `SeedNotRevealed`, `VrfCommitStale`, `VrfNotRevealable`, `VrfAlreadyCommitted`.

### Constants (`constants.rs`)

```rust
pub const DEFAULT_DISPUTE_WINDOW_SECS: u32 = 3_600;        // 1h
pub const MAX_DISPUTE_WINDOW_SECS: u32 = 86_400;            // 24h
pub const ORGANIZER_RESPONSE_WINDOW_SECS: i64 = 86_400;    // 24h
pub const STALE_VRF_SLOTS: u64 = 9_000;                    // ~1h on Solana — unrevealed commits expire
pub const SWITCHBOARD_ON_DEMAND_PROGRAM_ID: Pubkey =
    pubkey!("Aio4gaXjXzJNVLtzwtNVmSqGKpANtXhybbkhtAC94ji2");      // devnet
```

### Dependencies (`Cargo.toml`)

User-installed:
- `switchboard-on-demand = { version = "0.12.1", features = ["anchor", "devnet"] }` — required for VRF commit/reveal CPI and `RandomnessAccountData::parse(...).get_value(...)`.

Per the v1.1-plan precedent (line 190), list don't edit.

### Account-size migration

Existing MVP tournaments are organizer-only. After V1, they still work (the `OrganizerOnly` path is unchanged). However, both `Tournament` and `MatchNode` grow:
- Tournament: +45 bytes (`dispute_window_secs` +4, `vrf_randomness_account` +32, `vrf_commit_slot` +8, `seed_revealed` +1); zero/false defaults are safe for old organizer-only tournaments.
- MatchNode: +83 bytes for the proposal envelope; zero/`None` defaults are safe.

Add an idempotent `migrate_v1_tournament(ctx)` ix that anyone can call. It reallocs the Tournament and zero-fills the new fields. MatchNode realloc happens lazily — the first state-changing ix on a given match (`propose_result`, `report_result`, `confirm_result`) reallocs in-place. Same precedent as V1.2's planned migration ix.

Alternatively: fresh program ID redeploy (preferred for devnet — same trade-off documented in V1.1 line 195-198).

---

## SDK changes (`bracket-chain-sdk/`)

### Files to modify

1. **`src/idl/`** — regenerate via `pnpm run sync-idl` after program rebuild.
2. **`src/types.ts`** — add `ProposalSource`, `DisputeReasonCode`; extend `Tournament` (`disputeWindowSecs`), extend `Match` (`proposalSource`, `proposer`, `proposedWinner`, `proposedAt`, `claimDeadline`, `disputed`, `disputeReason`). Rename `SettlementMode::Manual` → `SettlementMode::OrganizerOnly`; add `SettlementMode::PlayerReported`.
3. **`src/errors.ts`** — typed errors for all 12 new program errors.
4. **`src/methods/createTournament.ts`** — extend params with `disputeWindowSecs?: number` (default 3600); validate range.
5. **`src/methods/reportResult.ts`** — for `PlayerReported` mode, throw `OrganizerCannotReportInPlayerModeError` pre-flight (don't waste a transaction).
6. **`src/methods/index.ts`** — export new methods.
7. **`src/index.ts`** — re-export.

### New method files

- **`src/methods/proposeResult.ts`** — mirrors `reportResult.ts` shape. Pre-flight checks: caller is one of the two players, no prior proposal exists, tournament is `PlayerReported` mode.
- **`src/methods/confirmResult.ts`** — pre-flight: caller is the counterparty, proposal exists and not disputed, deadline not passed. Handles final-vs-non-final advancement accounts same as `reportResult.ts:205-303`.
- **`src/methods/disputeResult.ts`** — pre-flight: caller is the counterparty, deadline not passed.
- **`src/methods/claimResult.ts`** — permissionless. SDK helper checks the deadline and warns if called too early.
- **`src/methods/resolveDispute.ts`** — organizer-only; only valid when `match.disputed == true`.
- **`src/methods/forceClaimDisputed.ts`** — permissionless; pre-flight checks the 24h organizer-response window has elapsed.
- **`src/methods/migrateV1Tournament.ts`** — idempotent migration helper.
- **`src/methods/requestSeed.ts`** — organizer-signed; allocates Switchboard `RandomnessAccountData`, commits slot. Pre-flight: `settlement_mode != OrganizerOnly`, no prior unrevealed commit.
- **`src/methods/revealSeed.ts`** — permissionless; reveals VRF and writes `tournament.seed_hash`. Pre-flight: commit slot has elapsed but not exceeded `STALE_VRF_SLOTS`.

### New file: `src/oracle/vrf.ts`

Thin wrapper around `@switchboard-xyz/on-demand`'s `Randomness` helper, exposing `requestRandomness(tournamentPda)` and `revealRandomness(tournamentPda)` that the method files consume. Lives in `src/oracle/` (new subdirectory) — V1.2 will add `feedFactory.ts` and `feedReader.ts` here alongside.

### `src/methods/startTournament.ts` (existing — modify)

- Gate on `tournament.seed_revealed == true` for non-`OrganizerOnly` tournaments; throw `SeedNotRevealedError` pre-flight.
- For VRF-seeded tournaments, derive `MatchInitDescriptor[]` deterministically from `tournament.seed_hash` via client-side Fisher-Yates before submitting.

### SDK ergonomics: `Match` state helpers

Add to `client.ts` or a new `src/match-state.ts`:

```ts
export type MatchEffectiveState =
  | 'AwaitingResult'
  | 'PendingConfirmation'
  | 'PastDeadline'
  | 'Disputed'
  | 'Finalized';

export function getMatchEffectiveState(
  match: Match,
  tournament: Tournament,
  nowSecs: number
): MatchEffectiveState { /* derives from the proposal envelope */ }

export function getMatchActions(
  match: Match,
  tournament: Tournament,
  viewerWallet: PublicKey | null,
  nowSecs: number
): MatchAction[];   // [ 'propose' | 'confirm' | 'dispute' | 'claim' | 'resolve' | 'force-claim' ]
```

This is the single source of truth for "what can this viewer do on this match right now." The frontend's `ReportResultModal` (and V1.2's replacement) consume this helper instead of re-deriving the state machine in JSX.

### Dependencies (user-installed)

- `@switchboard-xyz/on-demand` (3.x family) — VRF client.
- `@switchboard-xyz/protos` — protobuf types for randomness commit/reveal payloads.

---

## Indexer changes (`bracket-chain-indexer/`)

### `prisma/schema.prisma`

Extend `Match` model:
```prisma
proposalSource     Int       @default(0)  // ProposalSource enum
proposer           String?
proposedWinner     String?
proposedAt         DateTime?
claimDeadline      DateTime?
disputed           Boolean   @default(false)
disputeReasonCode  Int?
```

Extend `Tournament`:
```prisma
disputeWindowSecs     Int       @default(3600)
settlementMode        Int       @default(0)   // (already from V1.1)
vrfRandomnessAccount  String?
vrfCommitSlot         BigInt?
seedRevealed          Boolean   @default(false)
```

New table for dispute history (preserves audit trail since on-chain only stores current state):
```prisma
model Dispute {
  id                 String   @id @default(cuid())
  tournamentAddress  String
  round              Int
  matchIndex         Int
  disputer           String
  reasonCode         Int
  reasonNote         String?  // free-text from frontend; off-chain only
  disputedAt         DateTime
  resolvedAt         DateTime?
  resolver           String?
  resolutionSource   Int?     // 0=OrganizerManual, 1=TimedOut
  finalWinner        String?
  @@index([tournamentAddress, round, matchIndex])
}

model Proposal {
  id                 String   @id @default(cuid())
  tournamentAddress  String
  round              Int
  matchIndex         Int
  proposer           String
  proposedWinner     String
  source             Int      // ProposalSource enum
  proposedAt         DateTime
  claimDeadline      DateTime
  resolvedAt         DateTime?
  resolutionType     Int?     // 0=Confirmed, 1=Claimed, 2=Disputed
  @@index([tournamentAddress])
  @@index([proposedAt])
}
```

### `src/webhooks/helius-parser.service.ts`

Add handlers for `ResultProposed`, `ResultConfirmed`, `ResultDisputed`, `ResultClaimed`, `DisputeResolved`. Each one:
- Upserts the relevant `Match` row (proposal fields).
- Inserts a row into `Proposal` (on `ResultProposed`) or `Dispute` (on `ResultDisputed`) for history.
- Updates `Dispute` on `DisputeResolved`.
- The existing `MatchReported` handler stays unchanged — finalization comes through it as before.

### `src/reconciliation/reconciliation.service.ts`

Surface the new proposal/dispute fields when fetching MatchNode accounts during drift recovery.

### New module: `src/notifications/` (narrow kernel — extended by V1 webapp Phase B)

This plan ships the **narrow kernel** of the notifications module — dispute-flow events only (`ResultProposed`, `ResultDisputed`, `force_claim_disputed`-eligible). It is **not** the full event bus. The unified `NotificationDispatcher` covering `matchReady`, `payoutReceived`, `tournamentStarting`, `tournamentCancelled`, plus the dispute events from this plan, lives in **V1 webapp plan Phase B** (`bracketchain-v1-webapp-plan.md`), which marks itself as the canonical owner. This plan's kernel is **extended in-place by webapp Phase B**, not replaced — same module path, expanded scope. If webapp Phase B is descoped, the kernel here still works (dispute notifications only), but every plan that emits user-facing events after V1 (`SponsorshipInjected` in V2-A, `BadgeMinted` in webapp Phase D, tier-upgrade prompts in V2-C) loses its push delivery path. Build the kernel here cleanly so Phase B can extend without rewrite.

Per the V1 spec: *"Step 2: Player B notified."* The on-chain part doesn't notify anyone; the indexer is the source of push notifications.

- **`notifications.module.ts`**
- **`notifications.service.ts`** — subscribes to `ResultProposed`, `ResultDisputed`, `force_claim_disputed`-eligible events. Fans out to:
  - Web Push (via `web-push` lib + browser subscriptions stored per wallet).
  - Email (via SendGrid/Postmark — only for wallets that opted in with an email, deferred to social login work).
  - Webhook (for organizer-side automations).
- **`notifications.controller.ts`** — endpoints to register/unregister push subscriptions per wallet.

V1 scope: web push only. Email and webhook are optional / Phase-late.

### New cron: `src/oracle/auto-claim-cron.service.ts`

`@Cron` running every minute. Finds matches where:
- `claimDeadline < now`
- `disputed == false`
- `match.status == Active` (per fresh chain read)

…and submits `claim_result` ixs. Without this, the dispute-window UX is "the bracket sometimes gets stuck until someone manually clicks claim" — bad. With it, finalization is automatic at the deadline.

Mirror cron for `force_claim_disputed` (24h organizer-response window).

### New cron: `src/oracle/vrf-reveal-cron.service.ts`

`@Cron` running every minute. Finds Tournaments where:
- `vrfCommitSlot != null`
- `seedRevealed == false`
- current slot > `vrfCommitSlot` (reveal eligible)
- current slot < `vrfCommitSlot + STALE_VRF_SLOTS` (not yet expired)

…and submits `reveal_seed`. Same operational story as the auto-claim cron: without it, an organizer who walks away after `request_seed` would stall the tournament until manual intervention. With it, reveal is automatic within 60s of eligibility.

For expired commits (`current_slot > vrfCommitSlot + STALE_VRF_SLOTS`), the cron emits a warning metric so the operator can nudge the organizer to re-request — the tournament is stuck until `request_seed` is called again with a fresh commit.

### Environment vars

- `INDEXER_CLAIM_PAYER_KEYPAIR` — funded keypair for permissionless `claim_result`, `force_claim_disputed`, **and `reveal_seed`**. Cost is sub-cent per tx; budget ~1.5 SOL/month on devnet (claim + VRF combined).
- `WEB_PUSH_VAPID_PUBLIC` / `WEB_PUSH_VAPID_PRIVATE` — for browser push notifications.
- `SWITCHBOARD_QUEUE` — devnet queue pubkey (default `EYiAmGSdsQTuCw413V5BzaruWuCCSDgTPtBGvLkXHbe7`; verify against SDK at install time).

### Dependencies (user-installed)

- `web-push` (for browser notifications).
- `@nestjs/schedule` (probably already present).
- `@switchboard-xyz/on-demand` — VRF reveal cron uses the same client as the SDK.

---

## Frontend changes (`BracketChain-Frontend/`)

### Patterns to mirror

- Modal pattern in `features/tournament/view/CancelModal.tsx` for the new modals.
- Sonner toast pattern already in place.
- TanStack Query hook pattern for the new derived-state hooks.
- `features/tournament/view/ReportResultModal.tsx` is the file most affected — it currently assumes organizer-signed reports; in V1 it becomes either a player-propose UI or a counterparty-confirm/dispute UI depending on viewer + match state.

### Files to modify

1. **`types/tournament.ts`** — add `ProposalSource`, `DisputeReasonCode`, `MatchEffectiveState`; extend `Tournament` and `Match` types.
2. **`lib/indexerToTournamentState.ts`** — map new fields, derive `effectiveState` per match using `getMatchEffectiveState` from the SDK.
3. **`features/tournament/create/CreateTournament.tsx`** — add settlement-mode picker (OrganizerOnly / PlayerReported) and dispute-window slider (only shown when PlayerReported). Default to PlayerReported for V1.
4. **`features/tournament/steps/DetailsStep.tsx`** — surface the new fields.
5. **`features/tournament/steps/ValidateState.ts`** — validate `disputeWindowSecs` range.
6. **`features/tournament/view/BracketView.tsx`** — match tooltip shows `MatchStateBadge` (`Awaiting`, `Pending B's confirm`, `Past deadline`, `Disputed`, `Final`). Disputed matches highlighted in red.
7. **`features/tournament/view/TournamentSidebar.tsx`** — "Open Matches" list shows matches needing the viewer's action (propose, confirm, dispute) with red-dot badges. Start button gating: for non-`OrganizerOnly` tournaments, gate on `tournament.seedRevealed`. Three sub-states: "Request VRF" (if no commit yet, organizer-only CTA → `requestSeed`), "VRF reveal pending… (auto-completes within 1 min)" with a manual "Reveal now" fallback (anyone → `revealSeed`), or normal Start (after reveal).
8. **`features/tournament/view/ReportResultModal.tsx`** — restructure: dispatches on `getMatchActions(match, tournament, viewer, now)`. Renders one of:
   - `ProposeResultPanel` — if viewer is a player and no proposal yet.
   - `ConfirmOrDisputePanel` — if viewer is the counterparty.
   - `ClaimResultPanel` — if anyone is viewing past the deadline.
   - `ResolveDisputePanel` — if viewer is organizer and match is disputed.
   - `ForceClaimPanel` — if viewer is anyone and 24h past dispute, organizer unresponsive.
9. **`features/tournament/view/TournamentHeader.tsx`** — show dispute window in human terms ("Auto-finalizes 1h after report").

### New files

1. **`features/tournament/view/ProposeResultPanel.tsx`** — radio: "I won" / "My opponent won". Calls `proposeResult`. Toast.
2. **`features/tournament/view/ConfirmOrDisputePanel.tsx`** — "Player A reported they won. Confirm or dispute?" with countdown timer. Two CTAs.
3. **`features/tournament/view/DisputeReasonModal.tsx`** — second-step modal once "Dispute" is clicked; reason-code dropdown + free-text note (note goes only to indexer's `Dispute.reasonNote`).
4. **`features/tournament/view/ClaimResultPanel.tsx`** — "Deadline passed without confirmation. Anyone can finalize." Single CTA.
5. **`features/tournament/view/ResolveDisputePanel.tsx`** — organizer view: shows the dispute reason + note, radio for correct winner, calls `resolveDispute`.
6. **`features/tournament/view/ForceClaimPanel.tsx`** — "Organizer hasn't resolved this dispute in 24h. The proposed result will be accepted." Single CTA.
7. **`hooks/useMatchEffectiveState.ts`** — wraps `getMatchEffectiveState` from the SDK with TanStack Query (re-derives every second for active countdowns).
8. **`hooks/useViewerMatchActions.ts`** — `MatchAction[]` for the connected wallet on a given match.
9. **`hooks/usePushSubscription.ts`** — registers/unregisters browser push with the indexer.
10. **`hooks/useVrfStatus.ts`** — polls `tournament.seedRevealed` and `vrfCommitSlot`; derives the three Start-button sub-states (no commit / commit pending reveal / revealed). Falls back to a "VRF commit expired — re-request" state if `current_slot > vrfCommitSlot + STALE_VRF_SLOTS`.
11. **`components/MatchStateBadge.tsx`** — small reusable badge.
12. **`components/DisputeCountdown.tsx`** — countdown component (same shape V1.2 uses for oracle window — share this component now).
13. **`constants/disputeReasons.ts`** — reason-code enum + display labels (mirrored from indexer).

### Permissionless-action UX

`ClaimResultPanel` and `ForceClaimPanel` are technically callable by anyone, but the indexer cron will pick them up within 60s of eligibility. The frontend should still surface the CTA (for "I want to claim immediately, not wait for the cron") and show the cron's expected fire time as a passive countdown. Toasts on completion regardless of who triggered.

---

## Verification (end-to-end devnet smoke)

1. **Program tests** (`bracket-chain-programs/tests/`):
   - Happy path: create PlayerReported tournament, both players join, `propose_result` from A, `confirm_result` from B → bracket advances.
   - Confirm by non-counterparty (e.g., the proposer themselves or a third party): `NotCounterparty`.
   - Confirm after deadline: rejected with `DisputeWindowClosed`.
   - Dispute by non-counterparty: `NotCounterparty`.
   - Dispute after deadline: `DisputeWindowClosed`.
   - Claim before deadline: `DisputeWindowOpen`.
   - Claim happy path: deadline elapsed, anyone calls `claim_result` → match finalizes with proposed winner.
   - Disputed match: `propose_result` → `dispute_result` → `resolve_dispute(winner=B)` → bracket advances with B as winner even if proposed was A.
   - Organizer attempts `report_result` on PlayerReported tournament: `OrganizerCannotReportInPlayerMode`.
   - 24h organizer-unresponsive: dispute raised, organizer silent → `force_claim_disputed` after 24h elapsed → proposed winner becomes final, event marks source=TimedOut.
   - OrganizerOnly mode unchanged: `report_result` still works.
   - Settlement-mode mismatch: PlayerReported tournament + `propose_result` from non-player → `ProposerNotInMatch`.
2. **Indexer integration test**: spin up against devnet, exercise propose/confirm/dispute/resolve cycles, verify `Proposal` and `Dispute` rows are populated with correct resolution types and timestamps.
3. **Frontend smoke**:
   - Wizard: create PlayerReported tournament with 30-min dispute window.
   - Player A clicks "Report Winner" → sees `ProposeResultPanel`, picks self as winner.
   - Player B receives push notification, navigates to bracket → sees `ConfirmOrDisputePanel` with countdown.
   - B clicks "Confirm" → match finalizes, bracket re-renders.
   - Re-run with B clicking "Dispute" → reason modal → organizer view shows `ResolveDisputePanel` → organizer picks correct winner → finalizes.
4. **Cron test**: propose result, let deadline elapse without confirmation, verify cron fires `claim_result` within 60s.
5. **Notification test**: subscribe a browser to web push, verify push fires on `ResultProposed` targeting the counterparty wallet.
6. **VRF tests** (`bracket-chain-programs/tests/`):
   - Happy path: `request_seed` for a PlayerReported tournament, advance the slot, `reveal_seed` (permissionless) → `seed_hash` is non-zero, `seed_revealed == true`.
   - Reveal in same slot as commit → `VrfNotRevealable`.
   - Reveal after `STALE_VRF_SLOTS` → `VrfCommitStale`; subsequent `request_seed` to re-commit → success.
   - `start_tournament` for PlayerReported tournament without revealed seed → `SeedNotRevealed`.
   - `start_tournament` for OrganizerOnly tournament without VRF → success (existing implicit seeding).
   - Deterministic bracket: same `seed_hash` produces same Fisher-Yates ordering across two SDK invocations.
7. **VRF cron test**: `request_seed`, walk away, verify indexer cron fires `reveal_seed` within 60s and tournament moves to ready-to-start.
8. **VRF expiry alert test**: `request_seed`, fast-forward past `STALE_VRF_SLOTS`, verify cron emits expiry warning metric and does not attempt to reveal.

---

## Open questions to resolve before kickoff

1. **Dispute reason note storage.** On-chain only stores `reason_code: u8`; full reason text + screenshot URLs live in the indexer's `Dispute.reasonNote` (off-chain). For prize disputes worth thousands of dollars, is off-chain enough? **Recommendation**: keep off-chain for V1 (saves account-size and account-creation rent on every dispute); revisit if disputes become high-stakes. Off-chain doesn't mean unrecorded — the indexer keeps it permanently and includes it in CSV export.
2. **Should `dispute_result` extend the deadline?** Argument for: organizer needs time to investigate. Argument against: predictable deadlines are simpler. **Recommendation**: don't extend automatically; the 24h `force_claim_disputed` window gives a separate, longer clock for organizer action.
3. **Concurrent disputes from both players?** Edge case: A proposes, B disputes, but then A also wants to "dispute" (their own proposal was a typo). Cleanest UX: `propose_result` is irrevocable once submitted; if A typoed, A must wait for B to dispute and then for organizer to resolve. Alternative: a `withdraw_proposal` ix is cheap (~30 LOC). **Recommendation**: ship without `withdraw_proposal`; add later if user-tested as a real friction point.
4. **Should `claim_result` reward the claimer?** Argument for: incentivize bots to keep the bracket flowing. Argument against: indexer cron handles it anyway; bots would only race the cron. **Recommendation**: no reward in V1. If the indexer cron ever proves unreliable, revisit.
5. **VRF seeding interaction.** Resolved — moved to "Sequencing constraint (locked)" at the top of this plan. VRF ships here; V1.2 inherits, formats Phase C inherits, V2-C inherits. Do not extract.
6. **Rename `Manual` → `OrganizerOnly` in V1.1's enum.** Breaking IDL change. Acceptable on devnet (no real users). If V1.1 has already hit any kind of stable integration, weigh the cost. **Recommendation**: rename now — the longer it lives as `Manual`, the more places it'll get hardcoded.

---

## Explicitly out of scope (V1.2+)

- **Oracle-based proposals** (V1.2): `ProposalSource::Oracle` variant is reserved here but not implemented. V1.2 adds a new ix `propose_result_oracle` that's permissionless and validates a Switchboard pull feed instead of a player signer.
- **Game-server attestation** (V2): `ProposalSource::GameServer` variant is reserved.
- **Staked-arbiter dispute resolution** (V3): arbiters stake tokens, can be slashed for overturned rulings.
- **Cross-tournament reputation**: tracking player honesty across tournaments — interesting but separate concern.
- **Per-match dispute-window override**: V1 ships tournament-level window only; per-match override is a V1.3 nicety.
- **Replay-evidence URI on `Dispute`**: storing a Demos/Replay URL with the dispute. Off-chain (indexer table). Defer.
- **Sponsor/spectator-triggered disputes**: anyone-can-dispute, not just the counterparty. Out of scope (creates griefing surface).

---

## V1.2 layering preview (for context only — not implemented in this plan)

Once this plan ships, V1.2's oracle work collapses to:

1. New ix `propose_result_oracle(round, match_index)`: permissionless, reads the Switchboard PullFeed bound to the match (via the V1.2-only `bind_match_feed` ix), validates the returned identity hash against `Participant.identity_hash`, then **writes the same proposal envelope** this plan defines — with `proposal_source = ProposalSource::Oracle`.
2. The rest of the dispute flow (`confirm_result`, `dispute_result`, `claim_result`, `resolve_dispute`, `force_claim_disputed`) **works unchanged**. The oracle is just a new proposer.
3. The only V1.2-specific additions on `MatchNode` become: `commitment: Option<MatchCommitment>` and `switchboard_feed: Pubkey`. Everything else is already here.
4. VRF is **already in V1** — no `request_seed`/`reveal_seed` work in V1.2; the oracle path inherits the same seed-revealed bracket.

Net result: V1.2's program-side change shrinks from 9 new instructions to 4 (`commit_match_lobby`, `bind_match_feed`, `propose_result_oracle`, `set_oracle_config`, plus the `dispute_result` signer broadening and the `migrate_v1_2_tournament` adapter).

This is the architectural payoff for sequencing V1 first (and bundling VRF here rather than in V1.2).

---

## Critical files (quick reference)

**Program:**
- `bracket-chain-programs/programs/bracket-chain/src/lib.rs` — add 9 new ix entrypoints (`propose_result`, `confirm_result`, `dispute_result`, `claim_result`, `resolve_dispute`, `force_claim_disputed`, `request_seed`, `reveal_seed`, `migrate_v1_tournament`), extend `create_tournament`, `report_result`, and `start_tournament`
- `bracket-chain-programs/programs/bracket-chain/src/state/match_node.rs:10-22` — add the proposal envelope (7 new fields)
- `bracket-chain-programs/programs/bracket-chain/src/state/tournament.rs` — add `dispute_window_secs`, `vrf_randomness_account`, `vrf_commit_slot`, `seed_revealed`; extend `SettlementMode` (`Manual` → `OrganizerOnly`; add `PlayerReported`)
- `bracket-chain-programs/programs/bracket-chain/src/state/protocol_config.rs` — add `default_dispute_window_secs`, `max_dispute_window_secs`, `switchboard_program`, `switchboard_queue`
- `bracket-chain-programs/programs/bracket-chain/src/instructions/propose_result.rs` — new
- `bracket-chain-programs/programs/bracket-chain/src/instructions/confirm_result.rs` — new
- `bracket-chain-programs/programs/bracket-chain/src/instructions/dispute_result.rs` — new
- `bracket-chain-programs/programs/bracket-chain/src/instructions/claim_result.rs` — new
- `bracket-chain-programs/programs/bracket-chain/src/instructions/resolve_dispute.rs` — new
- `bracket-chain-programs/programs/bracket-chain/src/instructions/force_claim_disputed.rs` — new
- `bracket-chain-programs/programs/bracket-chain/src/instructions/request_seed.rs` — new
- `bracket-chain-programs/programs/bracket-chain/src/instructions/reveal_seed.rs` — new
- `bracket-chain-programs/programs/bracket-chain/src/instructions/start_tournament.rs` — gate on `seed_revealed`, derive bracket from `seed_hash` via Fisher-Yates
- `bracket-chain-programs/programs/bracket-chain/src/instructions/report_result.rs` — gate on settlement mode
- `bracket-chain-programs/programs/bracket-chain/src/instructions/create_tournament.rs` — extend params
- `bracket-chain-programs/programs/bracket-chain/src/instructions/migrate_v1_tournament.rs` — new
- `bracket-chain-programs/programs/bracket-chain/src/errors.rs` — add 16 variants (12 dispute + 4 VRF)
- `bracket-chain-programs/programs/bracket-chain/src/events.rs` — add 7 events (5 dispute + 2 VRF)
- `bracket-chain-programs/programs/bracket-chain/src/constants.rs` — add 5 constants (3 dispute + 2 VRF/Switchboard)
- `bracket-chain-programs/programs/bracket-chain/Cargo.toml` — add `switchboard-on-demand = "0.12.1"` dep

**SDK:**
- `bracket-chain-sdk/src/types.ts`, `errors.ts`, `index.ts` — extensions and rename
- `bracket-chain-sdk/src/methods/proposeResult.ts`, `confirmResult.ts`, `disputeResult.ts`, `claimResult.ts`, `resolveDispute.ts`, `forceClaimDisputed.ts`, `migrateV1Tournament.ts`, `requestSeed.ts`, `revealSeed.ts` — new
- `bracket-chain-sdk/src/methods/createTournament.ts`, `reportResult.ts`, `startTournament.ts` — extend
- `bracket-chain-sdk/src/match-state.ts` — new (effective-state helpers)
- `bracket-chain-sdk/src/oracle/vrf.ts` — new (`@switchboard-xyz/on-demand` wrapper)
- `bracket-chain-sdk/scripts/sync-idl.mjs` — run after program rebuild
- `bracket-chain-sdk/package.json` — add `@switchboard-xyz/on-demand` + `@switchboard-xyz/protos` deps

**Indexer:**
- `bracket-chain-indexer/src/app.module.ts` — wire `NotificationsModule`, `AutoClaimCronModule`, `VrfRevealCronModule`
- `bracket-chain-indexer/src/webhooks/helius-parser.service.ts` — handlers for 7 new events (5 dispute + 2 VRF)
- `bracket-chain-indexer/prisma/schema.prisma` — extend `Match` + `Tournament` (incl. VRF fields); new `Dispute`, `Proposal` tables
- `bracket-chain-indexer/src/reconciliation/reconciliation.service.ts` — surface new fields
- `bracket-chain-indexer/src/notifications/` — new module
- `bracket-chain-indexer/src/oracle/auto-claim-cron.service.ts` — new
- `bracket-chain-indexer/src/oracle/vrf-reveal-cron.service.ts` — new
- `bracket-chain-indexer/package.json` — add `@switchboard-xyz/on-demand` dep

**Frontend:**
- `BracketChain-Frontend/types/tournament.ts` — extend
- `BracketChain-Frontend/lib/indexerToTournamentState.ts` — map + derive effective state
- `BracketChain-Frontend/features/tournament/create/CreateTournament.tsx`, `steps/DetailsStep.tsx`, `steps/ValidateState.ts` — settlement-mode picker + dispute-window slider
- `BracketChain-Frontend/features/tournament/view/ReportResultModal.tsx` — restructure as action dispatcher
- `BracketChain-Frontend/features/tournament/view/ProposeResultPanel.tsx`, `ConfirmOrDisputePanel.tsx`, `DisputeReasonModal.tsx`, `ClaimResultPanel.tsx`, `ResolveDisputePanel.tsx`, `ForceClaimPanel.tsx` — new
- `BracketChain-Frontend/features/tournament/view/BracketView.tsx`, `TournamentSidebar.tsx` (incl. VRF gating + manual reveal CTA), `TournamentHeader.tsx` — extend
- `BracketChain-Frontend/hooks/useMatchEffectiveState.ts`, `useViewerMatchActions.ts`, `usePushSubscription.ts`, `useVrfStatus.ts` — new
- `BracketChain-Frontend/components/MatchStateBadge.tsx`, `DisputeCountdown.tsx` — new
- `BracketChain-Frontend/constants/disputeReasons.ts` — new

**Docs:**
- `bracketchain-main/README.md` — V1 setup section + MVP → V1 delta.
- `bracketchain-mvp-plan.md` — add Phase reference to this plan.
- `bracketchain-v1.1-plan.md` — note that the `Manual` → `OrganizerOnly` rename happens in V1; SAS work itself unaffected.
- `bracketchain-v1.2-plan.md` — needs revision (next plan) to collapse oracle-specific ixs onto this plan's proposal primitive.
