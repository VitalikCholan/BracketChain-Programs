# BracketChain V1 — Program Improvements (Custom Payouts + Close Tournament)

## Context

Two small program-side concerns surface from cross-checking the codebase against the §5.2 Account Structures spec and the existing frontend's promises:

1. **Custom payout structures.** The frontend's `types/tournament.ts` already declares `PayoutPreset = "wta" | "standard" | "deep" | "custom"` (line 8), but the program only honors the three presets — `"custom"` is a UI promise with no on-chain backing. Closing this `[[project_frontend_backend_gaps]]` item lets organizers specify arbitrary per-placement payout percentages at create time.

2. **`close_tournament` for rent reclaim.** After a tournament completes, the Tournament PDA + every MatchNode PDA + every Participant PDA holds rent (~0.002 SOL each) that's effectively dead. For a 128-player tournament that's ~0.25 SOL recoverable per tournament. At scale, real money. Spec §5.1 names `close_tournament` as a program ix; current program has no such ix.

Both are **surgical, additive, V1-orthogonal** changes — they don't require V1 player-reported, formats, V1.1, V1.2, or any other plan to have shipped first. They could be implemented in either order; this plan presents them as Phase A and Phase B for sequencing convenience.

**Scope decisions (locked):**

- **Phase A — Custom payouts.** New `PayoutPreset::Custom([u16; 8])` enum variant — fixed-size 8-slot basis-points array. Sum must equal 10_000 (100%). Eight slots covers all current presets (Deep is 7 placements + 1 spare) and matches the spec's `payout_structure: [u16; 8]`.
- **Phase B — Close tournament.** Permissionless ix; callable on any `status ∈ {Completed, Cancelled, PartialCancelled}` tournament. **No grace period** (close-immediately on completion is allowed). Reclaimed rent goes to the original organizer. The Tournament PDA + all MatchNode + Participant PDAs close in a single chunked operation.

**Out of scope** (recorded at the bottom): per-placement-cap rules (e.g., "1st place can't exceed 80%"), payout-preset versioning (e.g., backwards-compat for future Deep variants), grace-period close, configurable close-recipient (always organizer in V1), rent-reclaim for the singleton `ProtocolConfig` PDA.

---

# Phase A — Custom Payout Structures

**Ownership (locked).** This plan is the **canonical source of truth for custom payouts**. The V2 plan (`bracketchain-v2-plan.md`) lists "Custom payout curves" as Phase B but explicitly delegates to this section — V2 Phase B is a cross-reference, not net new work. If you are reading the V2 plan and seeing custom payouts there, **come back here**; the spec lives in this file. Custom payouts ship as part of V1, ahead of V2 sponsor + game-server attestation work, because (a) the frontend already advertises a `"custom"` PayoutPreset (closes a real frontend/backend gap), and (b) it's an additive enum variant with zero V1 dependency cost. Do not re-implement in V2.

## Why now

`types/tournament.ts:8` already commits to a `"custom"` PayoutPreset on the frontend. Either the frontend has been showing a UI promise the program can't fulfill (the `[[project_frontend_backend_gaps]]` situation), or the field is unused dead code. Phase A closes the gap by making the program honor custom payouts at create time.

## Design

### `PayoutPreset` enum extension

Current:
```rust
pub enum PayoutPreset {
    WinnerTakesAll,        // 100% to 1st
    Standard,              // 60/25/15 to 1st/2nd/3rd
    Deep,                  // 40/25/15/10/5/3/2 to 1st..7th
}
```

New:
```rust
pub enum PayoutPreset {
    WinnerTakesAll,
    Standard,
    Deep,
    Custom([u16; 8]),      // basis points per placement; sum must == 10_000
}
```

`★ Insight ─────────────────────────────────────`
The fixed-size `[u16; 8]` (rather than `Vec<u16>`) is deliberate. Variable-length payout arrays would force `Tournament` to use `InitSpace::INIT_SPACE` with a max-bound, which adds rent overhead for tournaments that don't use Custom anyway. The 8-slot fixed shape covers Deep (the widest existing preset) plus one spare. If a future tournament wants 16 placements, that's a new `CustomWide([u16; 16])` variant or a separate plan — neither breaks existing tournaments. Discriminator stability for the first three variants (`0`, `1`, `2`) is preserved; `Custom` is variant `3`.
`─────────────────────────────────────────────────`

Account-size cost: `+1 byte discriminator + 16 bytes payload = 17 bytes` per Tournament. Existing tournaments with `Standard` etc. don't pay this cost because Anchor's `InitSpace` computes the max of all variants — but it's a one-time +16 bytes for tournaments using `Custom`. Total Tournament account growth: 16 bytes worst-case.

### Validation rules (at `create_tournament`)

For `PayoutPreset::Custom(slots)`:

```rust
// 1. Sum must equal exactly 10_000 basis points (100%)
let total: u32 = slots.iter().map(|&x| x as u32).sum();
require!(total == 10_000, CustomPayoutSumInvalid);

// 2. Non-zero contiguous prefix — payouts cannot have gaps
let mut last_nonzero_idx = 0;
for (i, &slot) in slots.iter().enumerate() {
    if slot > 0 { last_nonzero_idx = i; }
}
for i in 0..=last_nonzero_idx {
    require!(slots[i] > 0, CustomPayoutHasGap);
}

// 3. At least one placement must be non-zero (else there's no payout)
require!(slots[0] > 0, CustomPayoutEmpty);

// 4. Placement count must not exceed bracket capacity
let placement_count = last_nonzero_idx + 1;
require!(placement_count as u16 <= max_participants, CustomPayoutTooManyPlacements);
```

The "no gaps" rule prevents weird payouts like `[50, 0, 50, 0, ...]` which would confuse UI and indexer logic.

### Payout distribution

In `report_result.rs`'s final-match branch (existing `distribute_prizes` logic), replace:

```rust
let percentages = match tournament.payout_preset {
    PayoutPreset::WinnerTakesAll => &[10000, 0, 0, 0, 0, 0, 0, 0],
    PayoutPreset::Standard       => &[6000, 2500, 1500, 0, 0, 0, 0, 0],
    PayoutPreset::Deep           => &[4000, 2500, 1500, 1000, 500, 300, 200, 0],
};
```

With:
```rust
let percentages: [u16; 8] = match tournament.payout_preset {
    PayoutPreset::WinnerTakesAll => [10000, 0, 0, 0, 0, 0, 0, 0],
    PayoutPreset::Standard       => [6000, 2500, 1500, 0, 0, 0, 0, 0],
    PayoutPreset::Deep           => [4000, 2500, 1500, 1000, 500, 300, 200, 0],
    PayoutPreset::Custom(slots)  => slots,
};
```

Existing `placement_payouts: Vec<PlacementPayout>` event field continues to work — it carries the resolved per-placement amounts regardless of preset.

### Errors

```rust
CustomPayoutSumInvalid,         // sum != 10_000
CustomPayoutHasGap,             // non-zero values not contiguous from index 0
CustomPayoutEmpty,              // all zeros
CustomPayoutTooManyPlacements,  // last non-zero index > max_participants - 1
```

### Events

**Event-versioning convention (locked in V1.1).** The `TournamentCreated` event below already carries `event_version: u8` as its first field (from V1.1's redeploy). This Phase A extension adds new trailing fields (`payout_preset_kind`, `payout_custom_slots`) — these are additive, so they do **not** trigger a version bump per V1.1's rule (additive trailing fields preserve version; reordering / type-changing fields bump). `TournamentClosed` (added by Phase B) follows the same convention with `event_version: u8` as first field.

`TournamentCreated` event gains the preset payload — currently emits `payout_preset: u8`; extend to:

```rust
#[event]
pub struct TournamentCreated {
    // ... existing fields ...
    pub payout_preset_kind: u8,           // 0=WTA, 1=Standard, 2=Deep, 3=Custom
    pub payout_custom_slots: [u16; 8],    // zero-filled for non-Custom presets
}
```

Indexer parses this; non-Custom tournaments show `payout_custom_slots = [0; 8]` (sentinel for "use the preset's hardcoded percentages"). Frontend can display either the preset name or the resolved percentages.

## SDK changes (Phase A)

1. **`src/types.ts`** — extend `PayoutPreset` union: `'WinnerTakesAll' | 'Standard' | 'Deep' | { Custom: number[] }`. Custom carries an 8-element number array.
2. **`src/methods/createTournament.ts`** — accept `customPayoutSlots?: number[]` param; validate length ≤ 8, sum === 10000, contiguous-from-zero. Pad to 8 with zeros for the on-chain shape.
3. **`src/errors.ts`** — typed errors for the 4 new program errors.
4. **`src/formats/payouts.ts`** (new) — helpers:
   - `payoutPresetToSlots(preset): number[]` — resolves any preset (including custom) to the 8-slot percentage array.
   - `slotsToReadable(slots, prizePool, mint): { placement, percentage, amount }[]` — UI formatter.
   - `validateCustomSlots(slots): { valid: boolean, error?: string }` — client-side pre-flight.

## Indexer changes (Phase A)

### `prisma/schema.prisma`

Extend `Tournament`:
```prisma
payoutCustomSlots    Int[]    @default([])  // 8-element array; empty for non-Custom presets
```

`PayoutPreset` enum gets a new variant:
```prisma
enum PayoutPreset {
  WinnerTakesAll
  Standard
  Deep
  Custom           // new
}
```

### `src/webhooks/helius-parser.service.ts`

Extend `TournamentCreated` handler: store `payoutCustomSlots` when `payout_preset_kind == 3`.

### `/tournaments/:address` response

Include `payoutPreset: 'WinnerTakesAll' | 'Standard' | 'Deep' | 'Custom'` and (when Custom) `payoutCustomSlots: number[]`. No new endpoint needed.

## Frontend changes (Phase A)

### Files to modify

1. **`types/tournament.ts`** — `PayoutPreset` is already `"wta" | "standard" | "deep" | "custom"`; align names with SDK (uppercase) or keep lowercase + map at SDK boundary. Plan: keep frontend lowercase as today; SDK exports a `payoutPresetToWire()` helper that handles the casing.
2. **`features/tournament/steps/PrizeStep.tsx`** — when `payoutPreset === 'custom'` is selected, render 8-slot input form with sliders or text fields. Live-validate sum = 100% via the SDK's `validateCustomSlots`.
3. **`features/tournament/create/CreateTournament.tsx`** — pass `customPayoutSlots` through to SDK `createTournament` call.
4. **`features/tournament/view/TournamentHeader.tsx`** — for Custom-preset tournaments, show the resolved breakdown ("1st: 50%, 2nd: 30%, 3rd: 20%") instead of just the preset name.

### Validation UX

The frontend should reject invalid sums before submission (e.g., live counter: "Total: 95% — must equal 100%"). Plan ships with a simple sum-display + submit-button gate; richer UX (auto-balance sliders, "split remaining equally" button) can land in a follow-up.

---

# Phase B — `close_tournament` for Rent Reclaim

**Ownership (locked) — B2C launch-critical, NOT optional polish.** With B2C as the target market (community tournaments, friend groups, streamer-organized events with small entry fees), `close_tournament` is the difference between BracketChain being economically viable and not. Without it, rent at create-time eats ~50% of the prize pool for typical 4-16 player tournaments — a non-starter for community adoption. This phase **must ship in Phase 1 redeploy bundle**, not deferred. It is not "operational hygiene at scale" — it is **make-or-break for the B2C use case**.

## Why now — rent economics post-V1 schema growth

Phase 1 redeploy (V1.1 + V1 player-reported + V1.2 + program-improvements + partial-cancel) significantly grows each account class:

| Account | MVP | After Phase 1 | Approx rent |
|---|---|---|---|
| Tournament (Anchor overhead included) | ~600 bytes | ~890 bytes | ~0.0031 SOL |
| Participant (per registrant) | ~80 bytes | ~155 bytes | ~0.0011 SOL |
| MatchNode (per match) | ~120 bytes | ~340 bytes | ~0.0024 SOL |
| Vault TokenAccount | ~165 bytes | ~165 bytes (unchanged) | ~0.0012 SOL |

**Concrete B2C scenarios (post-V1 redeploy, at $150/SOL):**

| Tournament shape | Total rent | USD cost | Typical entry fee | Rent vs. pool |
|---|---|---|---|---|
| 4-player friend group | ~0.014 SOL | $2.10 | $1 (pool $4) | **52%** |
| 8-player community | ~0.027 SOL | $4.05 | $1 (pool $8) | **51%** |
| 16-player community | ~0.055 SOL | $8.25 | $1 (pool $16) | **52%** |
| 64-player community | ~0.21 SOL | $31.50 | $1 (pool $64) | **49%** |
| 128-player max | ~0.42 SOL | $63 | $1 (pool $128) | **49%** |

For a $1-entry-fee community tournament, rent costs the organizer **roughly half the prize pool** up front. Without rent reclaim, that's a permanent loss that destroys the economics of small tournaments — exactly the B2C use case BracketChain targets.

**What `close_tournament` changes:**

After tournament finalization (`status ∈ {Completed, Cancelled, PartialCancelled}`), the indexer's `close-terminal.cron` waits 7 days (grace period for players to view results), then permissionlessly closes all participant/match/tournament accounts via chunked `close_tournament` calls. Rent flows back to the organizer's wallet.

**Net effect:** organizer pays rent **temporarily** (during active tournament + 7-day grace), then receives ~95% of it back. Net cost reduces from $2-$63 to ~$0.30 (indexer's gas costs only). This converts rent from a **permanent loss** to a **temporary float** — the economic difference that makes B2C-scale tournaments viable.

**Sequencing implication:** previously this plan listed Phase A (custom payouts) before Phase B (close_tournament). With B2C-pivot, both are equally critical and must ship in the Phase 1 program redeploy bundle alongside V1.1, V1 player-reported, V1.2, and partial-cancel. Do not defer Phase B to "post-launch optimization" — there is no "launch" without it for the B2C market.

## Pre-V1 rent figures (for comparison only)

| Account | Approx rent (devnet/mainnet) |
|---|---|
| Tournament (~300 bytes) | ~0.002 SOL |
| Participant (per registrant, ~80 bytes) | ~0.0009 SOL |
| MatchNode (per match, ~120 bytes) | ~0.0014 SOL |

For an 8-player single-elimination tournament: 1 + 8 + 7 = 16 accounts ≈ 0.024 SOL recoverable.

For a 128-player single elim: 1 + 128 + 127 = 256 accounts ≈ 0.34 SOL recoverable per tournament.

At pre-V1 schema sizes, rent was already significant; post-V1 schema growth roughly doubles the per-account size, pushing rent costs from "operational tax" to "blocker for small-pool tournaments." `close_tournament` lets anyone trigger the reclaim after the tournament terminates; rent flows back to the organizer (who paid it).

## Design

### New ix: `instructions/close_tournament.rs`

**Permissionless** — anyone can call. Inputs: chunk of accounts to close via `remaining_accounts`.

Handler:
```rust
require!(
    matches!(
        tournament.status,
        TournamentStatus::Completed | TournamentStatus::Cancelled | TournamentStatus::PartialCancelled
    ),
    TournamentNotTerminal,
);

// Close accounts in remaining_accounts.
for account_info in ctx.remaining_accounts {
    // Detect account type by data length + discriminator.
    if is_match_node(account_info) {
        close_account(account_info, organizer)?;
    } else if is_participant(account_info) {
        close_account(account_info, organizer)?;
    } else {
        return err!(InvalidAccountForClose);
    }
}

// Tournament closes only in the final call — chunk caller must pass the Tournament
// itself last, after all MatchNodes and Participants are closed.
// The `close = organizer` constraint on the Tournament account in the ix declaration
// handles the actual close transfer.
```

The Tournament account is declared as `#[account(mut, close = organizer)]` in the accounts struct, so Anchor handles the final rent transfer automatically when the ix returns successfully and Tournament is in the accounts list.

### Chunking

For 128p tournaments, 256 accounts × ~33 bytes per account ref = ~8.4 KB of account references in `remaining_accounts`. Fits in one tx with Address Lookup Tables, or chunks of ~20 accounts per ix without ALTs.

Pattern: SDK helper `closeTournament(client, pda)` batches into ~10-20 chunks, calls `close_tournament` ix for each chunk. Order: MatchNodes and Participants in any order, Tournament account in the final call (as `close = organizer`).

### Idempotency

Once an account is closed, it's gone — re-trying the same close on the same account fails because the account no longer exists. SDK helper handles this gracefully (treats "account not found" on retry as success).

### Authority

Plan: **permissionless** (anyone can call). The economic incentive: whoever calls pays tx fees but doesn't get the rent (organizer does). So in practice, the organizer or an automated cron driven by the indexer calls it.

Alternative considered: restrict to organizer. Rejected because (a) it's a low-stakes operation (rent goes to the right place regardless), and (b) automation via cron requires the indexer to call it — making it permissionless avoids the indexer needing organizer's signer.

### Errors

```rust
TournamentNotTerminal,     // status not in {Completed, Cancelled, PartialCancelled}
InvalidAccountForClose,    // account in remaining_accounts isn't a Tournament/Match/Participant PDA for this tournament
```

### Events

```rust
#[event]
pub struct TournamentClosed {
    pub tournament: Pubkey,
    pub organizer: Pubkey,
    pub total_rent_reclaimed: u64,    // lamports
    pub accounts_closed: u16,
}
```

Emitted in the final chunk (when the Tournament account itself closes). Indexer marks the Tournament row as `closed = true` and the row stays for historical querying; reading the on-chain Tournament account post-close returns "account does not exist" — indexer is the source of truth for closed tournaments.

## SDK changes (Phase B)

1. **`src/types.ts`** — no changes (no new account types, just an ix).
2. **`src/methods/closeTournament.ts`** (new) — batches close calls; uses indexer to enumerate the full set of MatchNode + Participant PDAs to close.
3. **`src/methods/index.ts`** — export.
4. **`src/errors.ts`** — typed errors.

## Indexer changes (Phase B)

### `prisma/schema.prisma`

Extend `Tournament`:
```prisma
closedAt           DateTime?
closedTxSig        String?
totalRentReclaimed BigInt?
```

### `src/webhooks/helius-parser.service.ts`

Handle new `TournamentClosed` event → set the new fields on the Tournament row.

### New cron: `src/cleanup/close-terminal.cron.ts`

`@Cron` running hourly. Finds tournaments where:
- `status ∈ {Completed, Cancelled, PartialCancelled}`
- `closedAt IS NULL`
- `completedAt < now - 7 days` (grace period before auto-close)

…and submits `close_tournament` chunks. Uses `INDEXER_CLEANUP_PAYER_KEYPAIR` (new — pays tx fees; recoups via the organizer's reclaimed rent? No — organizer keeps the rent, indexer pays the gas. Justification: the indexer benefits from smaller account-set queries, and the gas cost is sub-cent per tournament).

`★ Insight ─────────────────────────────────────`
The 7-day grace period is the load-bearing UX choice. Without it, a tournament completes → cron closes accounts within an hour → players who try to view the tournament page after the close see "account not found" RPC errors. With it, the tournament stays viewable on-chain for a week (typical retention window for "I want to see the bracket result"); after that, the indexer is the historical record. Players who want permanent provenance can query the indexer's `Payout` rows + final `Match` rows — those persist forever, only the on-chain accounts close.
`─────────────────────────────────────────────────`

The 7-day window is also configurable per environment via `CLOSE_TOURNAMENT_GRACE_DAYS` env var.

## Frontend changes (Phase B)

### Files to modify

1. **`features/tournament/view/TournamentHeader.tsx`** — for `closedAt != null`: show "Archived" badge; defer to indexer-cached data instead of trying to read on-chain Tournament account.
2. **`lib/sdk.ts`** — `loadView(pda)` falls back to indexer if RPC returns "account not found" (currently treats it as fatal). Pattern: if indexer has the tournament row, render from indexer-only data; show "Archived" UI affordance.

### New files

None. `close_tournament` itself has no user-facing flow — the cron handles it. Organizer dashboards may show "X tournaments archived, Y SOL reclaimed" as a stat (Phase B of the webapp plan's analytics section can pick this up).

### Verification of closed-tournament viewing

UI smoke: after a tournament closes, navigate to `/t/[address]`. Should still render with full bracket + payouts (from indexer), with an "Archived" header indicator. No RPC errors visible to the user.

---

## Verification (end-to-end devnet smoke)

### Phase A — Custom Payouts

1. **Program tests**:
   - Create tournament with `PayoutPreset::Custom([5000, 3000, 2000, 0, 0, 0, 0, 0])` → succeeds.
   - Create with sum = 9999 → `CustomPayoutSumInvalid`.
   - Create with `[5000, 0, 5000, 0, ...]` (gap) → `CustomPayoutHasGap`.
   - Create with all zeros → `CustomPayoutEmpty`.
   - Create with 8 placements but `max_participants = 4` → `CustomPayoutTooManyPlacements`.
   - Run a Custom-preset tournament to completion → verify payouts distribute per the custom slots, not the preset defaults.
   - Existing tournaments using Standard / Deep continue to work unchanged.
2. **Indexer integration**: Custom tournament's `TournamentCreated` event populates `payoutCustomSlots` correctly; `/tournaments/:address` returns the array.
3. **Frontend smoke**: create flow's Prize step renders 8-slot custom input when "Custom" selected; live sum-validator gates submit button; resulting tournament shows custom breakdown on header.

### Phase B — Close Tournament

1. **Program tests**:
   - 8-player tournament: complete normally → `tournament.status = Completed`. Run `close_tournament` chunks → all 16 PDAs close, organizer receives ~0.024 SOL refund.
   - Try `close_tournament` on `status == Active` → `TournamentNotTerminal`.
   - Try closing a MatchNode account from tournament A while passing Tournament A from tournament B → `InvalidAccountForClose`.
   - Idempotency: re-run close on already-closed accounts → SDK helper succeeds silently; no on-chain re-close.
2. **Indexer integration**: `TournamentClosed` event populates `closedAt`, `closedTxSig`, `totalRentReclaimed` on the Tournament row.
3. **Cron test**: a Completed tournament aged >7 days triggers auto-close within an hour of cron tick; verify all accounts close + reclaim amount matches expected.
4. **Frontend smoke**: post-close, navigate to `/t/[closed-tournament-address]` → page renders with indexer data + "Archived" badge; no RPC errors.

---

## Open questions to resolve before kickoff

### Phase A — Custom Payouts

1. **Per-placement minimum.** Should the program enforce a minimum percentage per non-zero slot (e.g., ≥1% per placement)? Argument for: prevents "dust" payouts that cost more in gas than they're worth. Argument against: organizer's choice. Recommendation: no minimum at the program level; let UI suggest.
2. **Symmetric to preset names: `Custom` vs custom-named.** Should organizers be able to name their custom preset? E.g., `payoutPresetName: Option<[u8; 32]>`. Adds 32 bytes to Tournament for a cosmetic field. Recommendation: skip for V1; SDK + frontend can label as "Custom" everywhere.
3. **Validation strictness on update.** Tournament's payout_preset is immutable post-create (today's MVP behavior). Should an admin override allow changing it for in-flight tournaments? Recommendation: no — payout immutability is a trust property organizers signal at create time.
4. **Should `[5000, 5000, 0, 0, 0, 0, 0, 0]` (50/50 split) be allowed?** Yes per the rules above (sum = 10000, contiguous, non-empty). Note: this means 1st and 2nd both receive identical payouts, which is unusual but valid. Recommendation: allow; flag in UI ("1st and 2nd will receive the same prize").

### Phase B — Close Tournament

5. **Grace period configurability per tournament.** Should an organizer be able to set a longer grace period at create time (e.g., "keep accounts open for 30 days")? Recommendation: V1 ships with one global grace (7 days, env-var-configurable); per-tournament override is a future "tournament archive policy" plan.
6. **Cron payer funding model.** Indexer pays tx fees (~0.000005 SOL per chunked close); organizer keeps the rent (~0.024-0.34 SOL). This is generous to organizers but the indexer's gas cost is negligible. If indexer-operating-cost becomes a concern, could split fees from rent (organizer pays cron's gas out of their reclaim). Recommendation: V1 ships with indexer-pays-gas — keeps the org UX clean; revisit if costs scale.
7. **Should `close_tournament` be callable by organizer only?** Plan says permissionless. Argument for permissionless: cron-driven without organizer signer. Argument against permissionless: opens a (low-stakes) griefing surface where someone closes a tournament the organizer wanted to keep viewable. Recommendation: keep permissionless; the grace period mitigates the griefing concern.
8. **Reclaim recipient.** Plan says organizer. Alternative: protocol treasury (treats rent as a protocol fee). Recommendation: organizer — they paid it, they get it back. Treasury-as-recipient is a different feature ("rent-as-protocol-revenue") and would need separate justification.

---

## Explicitly out of scope (follow-up plans)

- **Per-placement caps** (e.g., "1st place can't exceed 80%"). Could prevent organizer abuse where they self-tournament with `[9999, 1, 0, ...]`. Out of scope for V1.
- **Custom preset templates / shareable presets.** Organizers might want to save and re-use their custom configurations. Out of scope.
- **Payout-preset versioning.** Future Deep variants (Deep-12, Deep-16) would need a new variant or extended slots. Plan ships with `[u16; 8]` — wider arrays are a separate plan.
- **Grace-period close override.** Plan ships with single global 7-day grace. Per-tournament override is future work.
- **Configurable close recipient.** Always organizer in V1. Treasury-as-recipient is a separate "rent as protocol revenue" plan.
- **Rent-reclaim for the singleton `ProtocolConfig` PDA.** Never closes — it's protocol-global state.
- **Pre-emptive close of `Cancelled` (pre-start) tournaments.** Pre-start cancels already trigger refunds; close just happens via the cron. Could close immediately on cancel since no players have stake — minor optimization, defer.

---

## Critical files (quick reference)

**Program — Phase A:**
- `bracket-chain-programs/programs/bracket-chain/src/state/tournament.rs` — extend `PayoutPreset` enum with `Custom([u16; 8])`
- `bracket-chain-programs/programs/bracket-chain/src/instructions/create_tournament.rs` — validation rules for Custom variant
- `bracket-chain-programs/programs/bracket-chain/src/instructions/report_result.rs` — payout distribution reads slots from Custom variant
- `bracket-chain-programs/programs/bracket-chain/src/errors.rs` — 4 new error variants
- `bracket-chain-programs/programs/bracket-chain/src/events.rs` — extend `TournamentCreated` with `payout_custom_slots`

**Program — Phase B:**
- `bracket-chain-programs/programs/bracket-chain/src/instructions/close_tournament.rs` — new
- `bracket-chain-programs/programs/bracket-chain/src/events.rs` — `TournamentClosed`
- `bracket-chain-programs/programs/bracket-chain/src/errors.rs` — `TournamentNotTerminal`, `InvalidAccountForClose`
- `bracket-chain-programs/programs/bracket-chain/src/lib.rs` — add `close_tournament` entrypoint

**SDK:**
- `bracket-chain-sdk/src/types.ts`, `errors.ts` — extend
- `bracket-chain-sdk/src/methods/createTournament.ts` — accept + validate `customPayoutSlots`
- `bracket-chain-sdk/src/methods/closeTournament.ts` — new (Phase B)
- `bracket-chain-sdk/src/formats/payouts.ts` — new (Phase A; helpers for resolve/validate/format)

**Indexer:**
- `bracket-chain-indexer/prisma/schema.prisma` — extend Tournament with `payoutCustomSlots`, `closedAt`, `closedTxSig`, `totalRentReclaimed`; add `Custom` to `PayoutPreset` enum
- `bracket-chain-indexer/src/webhooks/helius-parser.service.ts` — handle extended `TournamentCreated` + new `TournamentClosed`
- `bracket-chain-indexer/src/cleanup/close-terminal.cron.ts` — new (Phase B)
- `bracket-chain-indexer/src/reconciliation/reconciliation.service.ts` — surface new Tournament fields

**Frontend:**
- `BracketChain-Frontend/types/tournament.ts` — verify `PayoutPreset` already includes `"custom"` (it does, line 8)
- `BracketChain-Frontend/features/tournament/steps/PrizeStep.tsx` — 8-slot custom input form
- `BracketChain-Frontend/features/tournament/create/CreateTournament.tsx` — pass `customPayoutSlots` through
- `BracketChain-Frontend/features/tournament/view/TournamentHeader.tsx` — render custom breakdown; render "Archived" badge for closed tournaments
- `BracketChain-Frontend/lib/sdk.ts` — graceful "account not found" fallback to indexer-only data

**Docs:**
- `bracketchain-main/README.md` — note custom payouts available in V1; note close-tournament + 7-day grace policy
- `bracketchain-mvp-plan.md` — flip the `payout_preset` mention from "3 presets" to "3 presets + Custom"; mention close-tournament under operational tooling
- Cross-plan refs: this plan is V1-orthogonal; no cross-plan dependencies to update.
