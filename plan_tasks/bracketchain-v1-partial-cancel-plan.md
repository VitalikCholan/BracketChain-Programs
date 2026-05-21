# BracketChain V1 — Partial Cancellation (Mid-Tournament)

## Context

The MVP's `cancel_tournament` is intentionally restricted to **pre-start** cancellations only (`status == Registration`). Once the bracket initializes and matches begin, cancellation is blocked. This is the right default for hackathon-scope: it keeps the prize-pool invariants simple (all entry fees still in the vault, no payouts yet, no half-finished bracket state).

But real esports has scenarios where mid-tournament cancellation is the right answer:
- Critical server outage in the game itself (Dota 2 servers down, Valorant patch breaks ranked).
- Organizer cannot continue (medical emergency, internet outage during a multi-day event).
- A discovered exploit / rule violation invalidates all matches played so far.
- Force-majeure events affecting the venue (LAN tournaments).

This plan adds **`partial_cancel_tournament`** — a new instruction that cancels an Active tournament and refunds participants per a clearly-defined policy. It does not change `cancel_tournament`'s pre-start semantics; the existing ix continues to work unchanged.

**Scope decisions (locked):**

- **Refund policy: "make-whole survivors."** All participants who have not yet been eliminated receive a **full entry-fee refund**. Eliminated participants receive nothing. The remaining vault balance (eliminated players' entry fees minus the protocol fee already escrowed) is returned to the organizer. Organizer deposit is also returned to organizer. This is the cleanest policy — equivalent to "cancel as if the tournament never happened" for survivors, while still letting the organizer recover their investment for matches that did complete.
- **Authority: organizer-signed.** Same auth model as `cancel_tournament`. No two-tier objection window — partial cancel is a destructive action, but it's the organizer's call (and they bear the reputational cost). If a future plan wants community oversight, that's a separate primitive.
- **Trigger window: any time during `status == Active`.** No round-boundary restriction. The ix is callable mid-round; in-flight matches' outcomes are discarded.
- **No bracket reactivation.** Once partial-cancelled, the tournament is terminal — no way to resume. Status flows to `PartialCancelled` (new variant) which is distinct from `Cancelled` for analytics/UI clarity.

**Out of scope** (recorded at the bottom): equal-split-among-survivors policy, bracket-position-weighted refund policy, partial refund to eliminated players, partial cancel with bracket reactivation, organizer-overridable refund policy at create time.

---

## Refund policy mechanics

For `N` registered participants paying `entry_fee` each:
- **Survivors** (uneliminated, `S` of them) → each gets `entry_fee` back.
- **Eliminated** (`N - S` of them) → get nothing.
- **Organizer** → gets back `organizer_deposit` + `(N - S) * entry_fee * (1 - protocol_fee_bps/10000)`.
- **Protocol** → keeps `(N - S) * entry_fee * protocol_fee_bps/10000` (the protocol fee on completed matches).

Vault invariant: `sum_of_refunds + organizer_deposit + protocol_fee_held == vault_balance` after the partial cancel.

`★ Insight ─────────────────────────────────────`
The "make-whole survivors" policy works because of a key property of single-elimination brackets: a participant is uneliminated *iff* they've won every match they've played. So the program can derive eliminated-vs-survivor status entirely from on-chain match results — it doesn't need to introduce a new "eliminated_at" field on `Participant` or trust an organizer-provided list. The derivation is `participant.losses == 0` — an O(1) byte-level check.
`─────────────────────────────────────────────────`

**Prerequisite (resolved in Phase 1 bundle).** This plan requires `Participant.losses: u8` to exist. That field is **cherry-picked from V1 formats plan Phase A and shipped in V1.1** (see V1.1 plan's `Modify: state/participant.rs` section — "Foundation stats" subsection). Together with `Participant.wins`, `points_for`, `points_against`, the four stats fields land in the V1.1 redeploy and are incremented by every match-finalization path (`report_result`, V1's `confirm_result`/`claim_result`/`resolve_dispute`/`force_claim_disputed`). With the stats fields available, `is_survivor()` is `participant.losses == 0` — a one-byte read per participant, the entire 128-player partial-cancel count phase fits in a single transaction with an Address Lookup Table. If for any reason the stats fields are not present (e.g., partial-cancel is back-ported to a program version without V1.1's Participant extension), the fallback derivation is to iterate MatchNode accounts and count losses per participant — 10× the CU budget, multi-chunk transaction flow. **Do not ship partial-cancel without the stats fields being available** — the fallback is functional but operationally painful.

---

## Architecture

```
                ┌──────────────────────────────────────────────┐
                │  BracketChain program                        │
                │                                              │
                │  partial_cancel_tournament (new)             │
                │   ↓                                          │
                │  1. Verify organizer + status == Active      │
                │  2. Set status = PartialCancelled            │
                │  3. Compute survivor list (losses == 0)      │
                │  4. Emit TournamentPartiallyCancelled        │
                │                                              │
                │  partial_refund_chunk (new — chunked)        │
                │   ↓                                          │
                │  Each call refunds up to ~10 participants.   │
                │  Idempotent via participant.refundPaid flag. │
                │  Last chunk: returns organizer surplus +     │
                │  deposit, emits TournamentRefundComplete.    │
                └──────────────────────────────────────────────┘
                                    │
                ┌───────────────────┴────────────────────┐
                ▼                                        ▼
        Indexer parses                            Frontend modal
        TournamentPartiallyCancelled +            walks user through
        per-chunk RefundIssued events             warning → confirm → progress
```

Chunking pattern mirrors V1.1's `cancel_tournament` for >32 participants. The ix is split into two phases:
1. **`partial_cancel_tournament`** — flips status, freezes bracket. One tx. Authority: organizer.
2. **`partial_refund_chunk`** — refunds N participants per call. Permissionless after status flip. Multiple calls process all survivors + eliminated; final call returns organizer surplus.

The two-phase split lets the status flip be atomic (no race where another tx writes to a bracket that's being cancelled) while letting refund execution scale with participant count.

---

## Program changes

### Modify: `state/tournament.rs`

Extend `TournamentStatus` enum:

```rust
pub enum TournamentStatus {
    Registration,
    PendingBracketInit,
    Active,
    Completed,
    Cancelled,
    PartialCancelled,    // new — mid-tournament cancellation
}
```

Discriminator `5` for `PartialCancelled`. Existing tournaments deserialize correctly because no existing tournament has status `5`.

Account-size impact: zero (enum discriminator is one byte, already allocated).

### Modify: `state/tournament.rs` — new field

```rust
pub partial_cancel_state: PartialCancelState,    // 0 bytes via Option-less encoding
```

```rust
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, InitSpace, PartialEq, Eq)]
pub struct PartialCancelState {
    pub initiated_at: i64,                       // 0 if not partial-cancelled
    pub survivor_count: u16,                     // computed at status-flip time
    pub eliminated_count: u16,
    pub refunds_issued: u16,                     // increments with each chunk
    pub organizer_surplus_returned: bool,        // idempotency for final chunk
}
```

Account-size impact: +13 bytes. Zero-initialized on existing tournaments via the migration ix.

### Modify: `state/participant.rs`

No new fields. Existing `refund_paid: bool` (from MVP) is reused for partial-cancel refunds — same idempotency mechanism.

### New ix: `instructions/partial_cancel_tournament.rs`

Organizer-signed. No inputs beyond accounts.

Handler:
```rust
require!(tournament.organizer == ctx.accounts.organizer.key(), Unauthorized);
require!(tournament.status == TournamentStatus::Active, NotActive);

// Count survivors by iterating Participant PDAs via remaining_accounts
let mut survivor_count: u16 = 0;
let mut eliminated_count: u16 = 0;
for participant_ai in remaining_accounts.iter() {
    let p = Participant::try_deserialize(...)?;
    require_keys_eq!(p.tournament, tournament.key(), WrongParticipant);
    if is_survivor(&p) {
        survivor_count += 1;
    } else {
        eliminated_count += 1;
    }
}
require!(survivor_count + eliminated_count == tournament.participant_count, ParticipantSetIncomplete);

tournament.status = TournamentStatus::PartialCancelled;
tournament.partial_cancel_state = PartialCancelState {
    initiated_at: Clock::get()?.unix_timestamp,
    survivor_count,
    eliminated_count,
    refunds_issued: 0,
    organizer_surplus_returned: false,
};

emit!(TournamentPartiallyCancelled {
    tournament: tournament.key(),
    organizer: tournament.organizer,
    survivor_count,
    eliminated_count,
});
```

**Survivor derivation:** `is_survivor(p)` returns `p.losses == 0`. If the formats plan hasn't shipped yet (so `Participant` doesn't have `losses`), the program reads each participant's `MatchNode` accounts to derive elimination status — slower path, but works.

For 128 participants, this ix needs all 128 Participant PDAs in `remaining_accounts`. That's ~10 KB of accounts data per tx. Fits within tx size if participants are passed via `address_table_lookup`, otherwise needs to be chunked. Plan: **require an Address Lookup Table** for `partial_cancel_tournament` calls with `participant_count > 32`. SDK helper constructs the ALT.

### New ix: `instructions/partial_refund_chunk.rs`

**Permissionless** — anyone can call. Inputs: chunk of participant PDAs via `remaining_accounts`, plus the vault + each participant's recipient token account.

Handler (per participant in chunk):
```rust
require!(tournament.status == TournamentStatus::PartialCancelled, NotPartialCancelled);
require!(!participant.refund_paid, AlreadyRefunded);

if is_survivor(&participant) {
    // CPI: vault → participant.wallet's ATA, amount = tournament.entry_fee
    transfer_from_vault(...)?;
}
// Eliminated participants get nothing but are still marked refundPaid for idempotency.
participant.refund_paid = true;
tournament.partial_cancel_state.refunds_issued += 1;

emit!(RefundIssued {
    tournament: tournament.key(),
    recipient: participant.wallet,
    amount: if is_survivor { tournament.entry_fee } else { 0 },
    refund_source: RefundSource::PartialCancel,
});
```

After all participants processed (`refunds_issued == participant_count`), the final chunk:
- Transfers `organizer_deposit + (eliminated_count * entry_fee * (10000 - protocol_fee_bps) / 10000)` to organizer.
- Sets `organizer_surplus_returned = true`.
- Emits `TournamentRefundComplete { tournament, total_refunded, organizer_returned, protocol_fee_held }`.

Chunked: 10 refunds per chunk fits comfortably in CU budget; ~13 chunks for 128 participants. Indexer cron (new — see below) drives this automatically.

### Modify: `instructions/report_result.rs`

Add status guard:
```rust
require!(
    tournament.status == TournamentStatus::Active,
    NotActive,  // existing error; partial-cancelled tournaments reject report attempts
);
```

This is the load-bearing safety check. Without it, a report tx submitted concurrently with the partial-cancel tx could mutate the bracket after status flipped.

### Modify: `instructions/cancel_tournament.rs`

No changes. Pre-start cancel works exactly as today.

### Modify: `errors.rs`

Add:
- `NotActive` — generalized name (already exists for some checks; rename if needed)
- `NotPartialCancelled` — `partial_refund_chunk` requires `PartialCancelled` status
- `AlreadyRefunded` — `participant.refund_paid` is true (existing)
- `ParticipantSetIncomplete` — `remaining_accounts` doesn't contain all participants

### Events

**Event-versioning convention (locked in V1.1).** Both new events below have `event_version: u8` as their first field (value `EVENT_VERSION_V1 = 1`), per the convention established in V1.1's `Modify: events.rs` section. The extension to `RefundIssued` (adding `refund_source: u8`) preserves the existing `event_version` first field — `RefundIssued` already carries it from the Phase 1 redeploy.

```rust
#[event]
pub struct TournamentPartiallyCancelled {
    pub event_version: u8,
    pub tournament: Pubkey,
    pub organizer: Pubkey,
    pub survivor_count: u16,
    pub eliminated_count: u16,
}

#[event]
pub struct TournamentRefundComplete {
    pub event_version: u8,
    pub tournament: Pubkey,
    pub total_refunded: u64,       // sum of survivor refunds
    pub organizer_returned: u64,   // surplus + deposit
    pub protocol_fee_held: u64,    // retained by treasury
}
```

`RefundIssued` is extended with a `refund_source: u8` field (`0 = FullCancel`, `1 = PartialCancel`). The existing indexer parser already handles `RefundIssued` for the pre-start cancel path; this just adds a discriminator.

### Constants

```rust
pub const PARTIAL_REFUND_CHUNK_SIZE: u16 = 10;
```

---

## SDK changes

### Files to modify

1. **`src/idl/`** — regenerate after program rebuild.
2. **`src/types.ts`** — add `'PartialCancelled'` to `TournamentStatus` union; add `PartialCancelState` type.
3. **`src/errors.ts`** — typed errors for `NotPartialCancelled`, `ParticipantSetIncomplete`.
4. **`src/methods/cancelTournament.ts`** — no change; pre-start cancel works unchanged.

### New files

1. **`src/methods/partialCancelTournament.ts`** — organizer-side. Constructs Address Lookup Table if `participant_count > 32`, fetches all Participant PDAs, builds + submits the ix.
2. **`src/methods/partialRefundChunk.ts`** — permissionless helper. Used both by an organizer driving manual chunks and by the indexer's auto-refund cron.
3. **`src/methods/queries.ts`** — extend with `getPartialCancelProgress(client, pda): { survivorCount, refundsIssued, totalRefunds, isComplete }` for UI progress.

### Dependencies (user-installed)

No new SDK deps.

---

## Indexer changes

### `prisma/schema.prisma`

Extend `Tournament`:
```prisma
partialCancelInitiatedAt    DateTime?
partialCancelSurvivorCount  Int?
partialCancelEliminatedCount Int?
partialRefundsIssued        Int       @default(0)
partialCancelComplete       Boolean   @default(false)
```

`TournamentStatus` enum already covers `Cancelled`; **add `PartialCancelled` as a new variant**:
```prisma
enum TournamentStatus {
  Registration
  PendingBracketInit
  Active
  Completed
  Cancelled
  PartialCancelled    // new
}
```

Extend `Payout` enum:
```prisma
enum PayoutKind {
  Prize
  Refund
  Fee
  OrganizerRefund
  PartialRefund        // new — distinguishes partial-cancel survivor refund from full cancel refund
}
```

### `src/webhooks/helius-parser.service.ts`

Add handlers for:
- `TournamentPartiallyCancelled` → flip Tournament.status, persist survivor/eliminated counts.
- `TournamentRefundComplete` → set `partialCancelComplete = true`.
- `RefundIssued` with `refund_source == PartialCancel` → insert Payout row with kind `PartialRefund`.

### New cron: `src/cancellation/partial-refund.cron.ts`

`@Cron` running every minute. Finds tournaments where:
- `status == PartialCancelled`
- `partialCancelComplete == false`

…and submits `partial_refund_chunk` ixs until all participants are refunded. Uses `INDEXER_REFUND_PAYER_KEYPAIR` (already funded for V1's auto-claim cron — reused here).

Without this, a partial-cancelled tournament's refunds would stall until the organizer manually chunks them. With it, refunds complete autonomously within ~10 minutes of the status flip (for 128-participant tournaments).

### `src/reconciliation/reconciliation.service.ts`

Surface partial-cancel fields when fetching Tournament accounts during drift recovery.

---

## Frontend changes

### Files to modify

1. **`types/tournament.ts`** — extend `TournamentStatus` union with `'PartialCancelled'`; extend `PayoutDistribution.kind` with `'PartialRefund'`.
2. **`lib/indexerToTournamentState.ts`** — map new fields.
3. **`features/tournament/view/TournamentSidebar.tsx`** — for `status == Active` and viewer is organizer: show "Partial Cancel" CTA below the existing "Report Result" button, styled as destructive (red).
4. **`features/tournament/view/TournamentHeader.tsx`** — for `status == PartialCancelled`: show banner with `partial_cancel_initiated_at` timestamp + refund progress.
5. **`features/tournament/view/CancelModal.tsx`** — if `status == Active`, route to the new `PartialCancelModal`; if `status == Registration`, keep existing pre-start cancel flow.

### New files

1. **`features/tournament/view/PartialCancelModal.tsx`** — multi-step modal:
   - Step 1: Warning — "This will cancel the tournament mid-play and refund uneliminated players. Eliminated players receive nothing. This action cannot be undone."
   - Step 2: Refund preview table — list of survivors (refunded) + eliminated (not refunded) + organizer surplus.
   - Step 3: Type "PARTIAL CANCEL" to confirm (matches industry pattern for destructive actions).
   - Step 4: Sign tx; show progress as refunds process via cron.
2. **`features/tournament/view/PartialRefundProgress.tsx`** — progress bar component used in modal step 4 + on the tournament page banner. Polls `getPartialCancelProgress` every 5s.
3. **`hooks/usePartialCancelProgress.ts`** — TanStack Query wrapper.

### Out of frontend scope

- Mid-round partial-cancel rules visualization (which match-in-progress gets discarded, etc.). Plan ships with no UI nuance for in-flight matches — they're simply void.
- "Reschedule" UX for re-running a partial-cancelled tournament with the same participants. Out of scope; users create a fresh tournament.

---

## Verification (end-to-end devnet smoke)

1. **Program tests** (`bracket-chain-programs/tests/`):
   - 8-player tournament, complete 2 rounds (leaves 2 survivors). Call `partial_cancel_tournament` → status flips, `survivor_count == 2`, `eliminated_count == 6`.
   - Call `partial_refund_chunk` for 10 participants → 2 survivors get `entry_fee`, 6 eliminated get 0, organizer gets surplus + deposit.
   - Idempotency: second call to `partial_refund_chunk` with same participants → `AlreadyRefunded`.
   - Status guard: try `report_result` on `PartialCancelled` tournament → `NotActive`.
   - Authority: non-organizer calls `partial_cancel_tournament` → `Unauthorized`.
   - Status guard: try `partial_cancel_tournament` on `Registration`-status tournament → `NotActive` (use full `cancel_tournament` for pre-start).
   - Survivor derivation: tournament where every participant has played and lost at least one match (only champion is survivor) → `survivor_count == 1`.
   - Edge case: tournament with 0 matches reported (just bracket-initialized) → all participants are survivors → all get full refunds (equivalent to pre-start cancel + protocol fee zero).
2. **Indexer integration test**:
   - Partial-cancel a tournament via direct ix; verify `TournamentPartiallyCancelled` updates DB; verify each `RefundIssued` creates a `Payout` row with kind `PartialRefund`.
   - Cron test: stop indexer mid-refund; restart; verify cron resumes refunding from where it left off.
3. **Frontend smoke**:
   - Start an 8-player tournament; complete 1 round; from organizer's wallet, navigate to tournament page → see Partial Cancel CTA.
   - Open `PartialCancelModal`; verify refund preview shows 4 survivors + 4 eliminated correctly.
   - Type confirmation phrase; sign tx; verify progress bar fills as cron processes refunds.
   - After all refunds: verify tournament header shows `PartialCancelled` banner with summary stats.
4. **Multi-wallet smoke test**:
   - 16 players join + start + complete rounds 1-2. Organizer partial-cancels. Verify within ~5 minutes all 4 survivors see their entry-fee refunds in their token accounts; 12 eliminated players see no change; organizer gets `12 * entry_fee * 0.965 + deposit`; treasury keeps `12 * entry_fee * 0.035`.

---

## Open questions to resolve before kickoff

1. **`PartialCancelled` vs reusing `Cancelled`.** Plan uses a distinct status variant for analytics + UI clarity. Alternative: collapse into `Cancelled` with a `partial_cancel: bool` flag. Recommendation: distinct status — clearer for indexer queries (`WHERE status = 'PartialCancelled'` vs joining flags), no downside.
2. **Should survivors receive *more* than their entry fee?** Some communities argue survivors should get a share of eliminated players' pool because they were closer to winning. Argument against: rewards luck of bracket position over skill. Argument for: incentivizes investment in continuing tournaments. Plan defaults to "make whole" (entry fee only); revisit if user research shows demand for "bonus refund."
3. **Address Lookup Table requirement at `participant_count > 32`.** Plan says yes. Alternative: chunk the count phase too (`partial_cancel_tournament_count_chunk` + `partial_cancel_tournament_finalize`). Recommendation: ALT — simpler, single-tx status flip, no race window.
4. **Protocol fee handling.** Plan keeps protocol fee proportional to eliminated entry fees only (matches "completed matches earned the fee"). Alternative: refund protocol fee entirely on partial cancel (treat partial cancel as "didn't really happen"). Recommendation: keep proportional — protocol did the work for completed matches; treasury earned that share.
5. **Reactivation / resume?** Plan says no — partial-cancelled is terminal. If demand emerges, a `resume_partial_cancelled` ix is conceivable but introduces multiple invariants to track (vault balance must be re-funded, status must reverse). Defer to a separate plan if needed.
6. **Cron payer funding.** Reuses `INDEXER_REFUND_PAYER_KEYPAIR` from V1 auto-claim cron. Budget impact: ~1-2 extra SOL/month on devnet (chunked refund txs are small). Acceptable.

---

## Explicitly out of scope (follow-up plans)

- **Equal-split-among-survivors refund policy.** Different stakeholder model; treat as a future "organizer-configurable refund policy" plan if demand emerges.
- **Bracket-position-weighted refund policy.** Same.
- **Partial refund to eliminated players.** Same.
- **Bracket reactivation** after partial cancel. Same.
- **Organizer-configurable refund policy at create time.** Currently policy is hardcoded; if multiple policies ship, add a `TournamentConfig.cancel_policy` enum. Future work.
- **Community / participant-triggered partial cancel.** E.g., majority-of-survivors signature triggers cancel. Different auth model; out of scope.
- **Mid-round-aware partial cancel** (special handling for matches in flight at cancel time). Plan treats in-flight matches as void. If "pending dispute window" matches need preservation, separate plan.

---

## Critical files (quick reference)

**Program:**
- `bracket-chain-programs/programs/bracket-chain/src/state/tournament.rs` — add `PartialCancelled` to `TournamentStatus` enum; add `PartialCancelState` struct + field
- `bracket-chain-programs/programs/bracket-chain/src/instructions/partial_cancel_tournament.rs` — new
- `bracket-chain-programs/programs/bracket-chain/src/instructions/partial_refund_chunk.rs` — new
- `bracket-chain-programs/programs/bracket-chain/src/instructions/report_result.rs` — status guard
- `bracket-chain-programs/programs/bracket-chain/src/events.rs` — `TournamentPartiallyCancelled`, `TournamentRefundComplete`; extend `RefundIssued` with `refund_source`
- `bracket-chain-programs/programs/bracket-chain/src/errors.rs` — `NotPartialCancelled`, `ParticipantSetIncomplete`
- `bracket-chain-programs/programs/bracket-chain/src/constants.rs` — `PARTIAL_REFUND_CHUNK_SIZE`

**SDK:**
- `bracket-chain-sdk/src/types.ts`, `errors.ts`, `index.ts` — extensions
- `bracket-chain-sdk/src/methods/partialCancelTournament.ts`, `partialRefundChunk.ts` — new
- `bracket-chain-sdk/src/methods/queries.ts` — `getPartialCancelProgress`

**Indexer:**
- `bracket-chain-indexer/prisma/schema.prisma` — `PartialCancelled` enum variant + Tournament fields + `PartialRefund` PayoutKind
- `bracket-chain-indexer/src/webhooks/helius-parser.service.ts` — handlers for 2 new events + extended `RefundIssued`
- `bracket-chain-indexer/src/cancellation/partial-refund.cron.ts` — new
- `bracket-chain-indexer/src/reconciliation/reconciliation.service.ts` — surface new fields

**Frontend:**
- `BracketChain-Frontend/types/tournament.ts` — extend
- `BracketChain-Frontend/lib/indexerToTournamentState.ts` — map
- `BracketChain-Frontend/features/tournament/view/TournamentSidebar.tsx` — Partial Cancel CTA
- `BracketChain-Frontend/features/tournament/view/TournamentHeader.tsx` — partial-cancelled banner
- `BracketChain-Frontend/features/tournament/view/CancelModal.tsx` — route by status
- `BracketChain-Frontend/features/tournament/view/PartialCancelModal.tsx`, `PartialRefundProgress.tsx` — new
- `BracketChain-Frontend/hooks/usePartialCancelProgress.ts` — new

**Docs:**
- `bracketchain-main/README.md` — V1 partial-cancel section under "Cancellation behaviors."
- `bracketchain-mvp-plan.md` — note that the §"Cancellation & Refund" flow now has a mid-tournament variant landing in this plan.
- `bracketchain-v1-player-reported-plan.md` — note the `report_result` status guard extension (rejects on `PartialCancelled`).
