# BracketChain V1 — Tournament Format Expansion (RR / DE / Swiss)

## Context

BracketChain's MVP shipped on devnet at `AuXJKpuZtkegs2ZSgopgckhN7Ev8bUz4zBc238LD2F1` with **single-elimination only**. The frontend already has UI for double-elim, Swiss, and round-robin (per `[[project_v1_format_expansion]]`) — the gap is entirely on-chain. This plan closes that gap.

**Strategic positioning:** this is the on-chain follow-up that lets the frontend's existing format-picker actually create non-single-elim tournaments. Single-elim is the lowest-skill-ceiling format; competitive communities expect at least DE (double-elim is the default for fighting-game and Smash tournaments) and Swiss (the default for Magic, chess, and trading-card games). RR is a smaller niche but useful for friend-group leagues.

**Mixed-prerequisite scope decision (locked).** Phases A and B stand alone — they do **not** depend on `bracketchain-v1-player-reported-plan.md`. Settlement stays organizer-reported across RR and DE. **Phase C (Swiss) requires V1's VRF surface** (`request_seed` / `reveal_seed` / `tournament.seed_hash` / `seed_revealed`) — Swiss uses VRF for round-1 pairing entropy. If V1 player-reported settlement ships alongside or after this plan, format-aware tournaments inherit the proposal envelope automatically — no rework on the format side, because `propose_result` / `confirm_result` / `claim_result` operate on `MatchNode` regardless of which bracket it lives in.

**Sequencing dependency for Phase C.** Swiss cannot ship until V1 player-reported plan has landed — VRF is the load-bearing primitive (without VRF-revealed seed, round-1 Swiss pairings are organizer-manipulable in the same way as MVP single-elim seeding). Practical implication: phases A → B can ship in any order relative to V1 player-reported; **phase C must ship after**. The canonical ship order is therefore: V1.1 → V1 player-reported (incl. VRF) → formats A+B → formats C. If V1 player-reported delays for any reason, formats A+B can still ship; only C is blocked.

**Phase order (cheap → expensive):**
- **Phase A — Round Robin.** Deterministic pairing, no VRF, fixed shape. Adds the format infrastructure (`TournamentFormat` enum, `bracket: u8` PDA seed discriminator) that B and C reuse. Also adds per-match score reporting (`score_a` / `score_b`) used by RR's differential tiebreaker.
- **Phase B — Double Elimination.** Graph-topology change (winners/losers/grand brackets). Adds loser-advancement plumbing in `report_result`.
- **Phase C — Swiss.** Per-round pairing requires a new `start_round` ix. Adds opponent-history tracking and a Buchholz-tiebreaker finalization step. **Depends on V1's VRF** — `pairing_seed` is the VRF-revealed `tournament.seed_hash`, not organizer-submitted entropy.

**Scope (locked):**
- Round Robin up to **16 players** (120 matches; cap exists because per-tournament match count is `N*(N-1)/2`).
- Double Elimination up to current 128-player cap, with optional grand-final reset.
- Swiss configurable rounds 3-9, no per-tournament participant cap beyond existing `[2, 128]`.
- Tiebreakers: RR uses wins → head-to-head → point differential; Swiss uses wins → Buchholz → join order.

**Out of scope** (recorded at the bottom): third-place playoff in DE, custom Swiss pairing systems (Dutch vs Monrad), bye-balancing for odd Swiss player counts (V1 handles odd counts via auto-bye to the lowest-ranked player; configurable later). Per-match score reporting **is in scope** (required for RR's differential tiebreaker; reused as a no-op for the other formats).

---

## Cross-cutting design

**Schema-prep already shipped in V1 player-reported redeploy.** The infrastructural changes that previously lived here — `TournamentFormat` enum, `bracket: u8` PDA seed discriminator, `score_a` / `score_b` on MatchNode, `format: TournamentFormat` on Tournament — were **cherry-picked into V1 player-reported plan's Phase 1 bundle** (see that plan's `Modify: state/match_node.rs` section, "schema-prep" subsection). The rationale: V1 player-reported's redeploy is already breaking MatchNode struct layout via the proposal envelope, so folding in the seed change at the same time pays the breaking-change cost once instead of twice.

By the time this formats plan lands, the schema is already in place:

| Already in-chain after Phase 1 | Description |
|---|---|
| `TournamentFormat` enum | `SingleElim` enabled, others gated with `FormatNotYetSupported` error |
| `Tournament.format: TournamentFormat` | Default `SingleElim`; settable at create_tournament time |
| MatchNode PDA seed `[b"match", tournament, bracket, round, match_index]` | All matches have `bracket = 0` until Phase B (DE) ships |
| `MatchNode.score_a`, `score_b` | Zero for unreported; populated by report_result if non-zero |
| `Participant.wins`, `losses`, `points_for`, `points_against` | Cherry-picked into V1.1 redeploy; incremented on every match finalization |

This formats plan therefore is **purely additive logic** — no schema changes, no PDA seed changes, no realloc. Phase A, B, C each lift `FormatNotYetSupported` for their variant and add the format-specific bracket-init / advancement / finalization logic.

### 1. `TournamentFormat` enum (pre-shipped in V1 redeploy)

```rust
pub enum TournamentFormat {
    SingleElim,    // 0 — Phase 1 enabled
    DoubleElim,    // 1 — Phase B enables
    Swiss,         // 2 — Phase C enables
    RoundRobin,    // 3 — Phase A enables
}
```

`★ Insight ─────────────────────────────────────`
Putting `SingleElim` at discriminator `0` is deliberate. Anchor's `default()` on `Pod` types is zero-bytes, so any field with `TournamentFormat::default()` reads as `SingleElim`. This means **tournaments created before formats logic ships deserialize correctly** — the `format` field zero-fills to single-elim, which is what they actually are. The variant ordering and discriminator stability is locked at V1 redeploy time; this plan only flips the runtime gate.
`─────────────────────────────────────────────────`

### 2. `MatchNode` PDA seed (pre-shipped in V1 redeploy)

Seed shape is **already** `[b"match", tournament, bracket: u8, round: u8, match_index_le_bytes: u16]` — set in V1 player-reported redeploy.

Per-format usage when this plan lands:
| Format | `bracket` values | Meaning |
|---|---|---|
| SingleElim | `0` only | (only bracket — current Phase 1 behavior) |
| RoundRobin | `0` only | (only bracket — Phase A logic) |
| Swiss | `0` only | (only bracket — Phase C logic) |
| DoubleElim | `0`, `1`, `2` | Winners / Losers / Grand — Phase B logic |

No PDA reseed needed at this plan's time. SDK `findMatchPda(tournament, bracket, round, matchIndex)` already exists from V1 SDK.

### 3. Tournament status flow — minor extension

Current: `Registration → PendingBracketInit → Active → Completed | Cancelled`.

**No change for RR or DE.** Swiss adds an intermediate "round-active" state encoded as a `current_round: u8` field on Tournament, with `status == Active` covering all round transitions. No new enum variants.

### 4. Migration ix

No migration ix needed — schema is already in place from V1 redeploy. Phase A, B, C each only add new ix dispatching for their format variant; they don't realloc Tournament or MatchNode.

### Account-size cost summary

| Account | Phase A | Phase B | Phase C | Cumulative (this plan) |
|---|---|---|---|---|
| Tournament | 0 (`format` shipped in V1) | +1 (`grand_final_reset`) | +2 (`rounds: u8` + `current_round: u8` + `round_active: bool`; `pairing_seed` is V1's `seed_hash`) | +3 bytes |
| MatchNode | 0 (`bracket` + `score_*` shipped in V1) | +10 (winner_dest + loser_dest) | 0 | +10 bytes |
| Participant | 0 (`wins`/`losses`/`points_*` shipped in V1.1) | 0 | +5 (`byes_received: u8` + `current_round_match: u32`) | +5 bytes |
| SwissPairingHistory (new) | — | — | +variable (one PDA per Swiss participant) | new account type |

Total schema growth from this plan is now **+18 bytes Tournament/MatchNode/Participant combined** (down from +34 in original plan) — most of the foundation was pre-shipped.

For a 32-player DE tournament (~62 matches), MatchNode growth alone is ~560 bytes. For a 16-player RR (120 matches), it's ~1.1 KB. Trivial in absolute terms; flagged here for the cumulative table only.

### Architecture diagram

```
                       ┌────────────────────────────────────────┐
                       │  BracketChain program (devnet)         │
                       │                                        │
                       │  Tournament.format: TournamentFormat   │
                       │  MatchNode seeded with bracket: u8     │
                       │                                        │
                       │  ┌─ SingleElim path (unchanged) ─┐    │
                       │  ├─ RoundRobin path (Phase A)    ─┤    │
                       │  ├─ DoubleElim path (Phase B)    ─┤    │
                       │  └─ Swiss path (Phase C)         ─┘    │
                       │                                        │
                       │  Shared: escrow vault, payout preset,  │
                       │          organizer-reported results,   │
                       │          cancel/refund flow            │
                       └────────────────────────────────────────┘
                                  ▲                ▲
                                  │ writes         │ reads
                                  │                │
                       ┌──────────┴────┐   ┌──────┴──────────────┐
                       │ SDK methods   │   │ Indexer:            │
                       │ extended for  │   │ - format-aware      │
                       │ format param  │   │   bracket parser    │
                       │ + new helpers │   │ - standings calc    │
                       │   per format  │   │   for Swiss/RR      │
                       └───────────────┘   └─────────────────────┘
                                  ▲                ▲
                                  │                │
                                  │      ┌─────────┴─────────────┐
                                  └──────┤ Frontend:             │
                                         │ - format picker (live)│
                                         │ - BracketView gets    │
                                         │   3 new layouts       │
                                         │ - Standings panel for │
                                         │   Swiss/RR            │
                                         └───────────────────────┘
```

The escrow + payout + cancel + organizer-deposit machinery is fully reusable across all four formats. The format dimension is purely in how matches relate to each other (the bracket graph) and how the final ranking is computed.

---

# Phase A — Round Robin

## Why RR first

Round Robin is the cheapest format to add: pairings are deterministic (Circle method), there's no random seeding, all matches are pre-known at `start_tournament`, and the final ranking is a single pass over all `MatchNode` accounts at finalization time. It's the format that exercises the `TournamentFormat` enum + `bracket: u8` seed without adding any bracket-graph complexity. Ship RR first to land the format infrastructure; B and C then become incremental.

## Pairing math (Circle method)

For `N` players (even), there are `N-1` rounds with `N/2` matches each. For odd `N`, one bye per round, so `(N-1)/2` actual matches per round across `N` rounds.

```
Round r, slot s: match between player[fixed=0] and player[rotate(r, s)]
                 plus  player[rotate(r, s')] vs player[rotate(r, s'')] for s' + s'' = N - 1
```

V1 caps RR at 16 players (always even). 15 rounds × 8 matches/round = **120 matches per RR tournament.**

The pairing is fully deterministic from join order. No VRF dependency.

## Program changes

### Modify: `state/tournament.rs`

```rust
pub format: TournamentFormat,    // already added in Phase A cross-cutting
```

No RR-specific fields. RR uses `participant_count` to derive everything else.

### Modify: `state/match_node.rs`

```rust
pub bracket: u8,                 // already added in Phase A cross-cutting
pub score_a: u16,                // games/points scored by player_a; 0 for unreported
pub score_b: u16,                // games/points scored by player_b; 0 for unreported
```

For RR: `bracket = 0` always; `score_a` / `score_b` carry the match outcome used by the differential tiebreaker. For SingleElim / DE / Swiss, scores are accepted but optional — they're stored if reported (useful for richer match views) and ignored by the bracket engine (winner is the only field that drives advancement).

### Modify: `state/participant.rs` — **no changes here, fields ship in V1.1**

The four stats fields:

```rust
pub wins: u8,
pub losses: u8,
pub points_for: u32,             // cumulative score across all reported matches; RR differential = points_for - points_against
pub points_against: u32,
```

…are **cherry-picked into V1.1's Participant struct** (see V1.1 plan's `Modify: state/participant.rs` → "Foundation stats" subsection). They ship in the V1.1 redeploy because three plans consume them — partial-cancel (`losses == 0` survivor check), this formats plan (RR/Swiss tiebreakers), and webapp Phase D (badge eligibility). Shipping them once in V1.1 avoids three separate Participant-struct extensions across plan rollout.

By the time this formats Phase A lands, the fields already exist and are being incremented on every match finalization by V1 player-reported's settlement code. This plan's job is just to **read** them for RR finalization — no struct extension needed. `wins` / `losses` drive the primary tiebreaker; `points_for` / `points_against` drive the differential tiebreaker. Overflow-safe (u32 caps 65535 games × 65535 points/game — way beyond any realistic tournament).

### Modify: `instructions/create_tournament.rs`

Extend signature with `format: TournamentFormat`. Validation:
- If `format == RoundRobin`, enforce `max_participants <= MAX_RR_PARTICIPANTS` (16) → `RoundRobinParticipantCapExceeded`.
- `payout_preset` validation: RR allows all three presets (WTA / Standard / Deep). Deep is the most natural for RR (top 7 placements paid).

Update `TournamentCreated` event to include `format`.

### Modify: `instructions/start_tournament.rs`

The current `start_tournament` is `SingleElim`-shaped: it initializes a binary-tree bracket. For RR:

- Skip `seed_hash` capture (no random seeding).
- Initialize all `120` (or fewer for N<16) `MatchNode` accounts via Circle-method pairing.
- Same chunking pattern as single-elim: 7 inits per chunk → ~18 chunks for 120 matches. Idempotent.

The match-init logic dispatches on `tournament.format`:

```rust
match tournament.format {
    TournamentFormat::SingleElim => init_single_elim_chunk(...)?,
    TournamentFormat::RoundRobin => init_round_robin_chunk(...)?,
    TournamentFormat::DoubleElim => init_double_elim_chunk(...)?,   // Phase B
    TournamentFormat::Swiss => return err!(SwissUsesStartRound),    // Phase C
}
```

Swiss diverges (see Phase C); the other three pre-init all matches.

### Modify: `instructions/report_result.rs`

Extend ix params with `score_a: u16, score_b: u16`. Validation:
- For `RoundRobin`: `score_a + score_b > 0` and `score_a != score_b` (RR matches cannot end tied — winner is determined by score). The `winner` param must agree with which score is higher → `RoundRobinScoreInconsistent`.
- For `SingleElim` / `DoubleElim` / `Swiss`: scores accepted but `(0, 0)` is allowed (organizer didn't track scores). If both non-zero, `winner` must still agree with higher score (defensive).

Dispatch on format:
- `SingleElim`: unchanged advancement; persist scores to `MatchNode`; increment `points_for` / `points_against` on both participants.
- `RoundRobin`:
  - Validate match Active, winner ∈ {a, b}, scores consistent with winner.
  - Mark match Completed; set `winner`, `score_a`, `score_b`.
  - Increment `participant.wins` for winner, `participant.losses` for loser.
  - Update `points_for` / `points_against` for both participants.
  - **No advancement** — RR matches don't feed into other matches.
  - If `tournament.matches_reported == tournament.total_matches`, call `finalize_round_robin(...)` to compute standings + distribute prizes.

### New ix: `instructions/finalize_round_robin.rs`

Called inline from `report_result` when the last match is reported. Organizer-signed via the parent `report_result` signer.

Handler:
1. Iterate all Participant PDAs for this tournament (via `remaining_accounts`).
2. Compute per-participant `differential = points_for - points_against` (signed i64 to avoid underflow).
3. Sort by `(wins DESC, head_to_head DESC, differential DESC, join_order ASC)`. The `join_order` fallback only fires for true ties across all three primary criteria.
4. Map ranks to `placement_payouts` per the chosen preset.
5. Distribute prizes via the existing payout logic + take 3.5% protocol fee.
6. Set `tournament.champion = ranked[0].wallet`, `tournament.status = Completed`.
7. Emit `TournamentCompleted` with the same `placement_payouts: Vec<PlacementPayout>` shape the indexer already parses.

`★ Insight ─────────────────────────────────────`
Reusing `TournamentCompleted`'s `placement_payouts` envelope is the load-bearing choice that makes the indexer format-agnostic. The indexer doesn't need to know whether placements came from a binary-tree advancement or an RR standings sort — it just reads the event and writes Payout rows. Same pattern V1's plan flagged as "the contract the indexer's lean parser depends on" (V1 line ~345).
`─────────────────────────────────────────────────`

The head-to-head tiebreaker requires reading the specific `MatchNode` between two tied players. For a clean implementation, pass match PDAs via `remaining_accounts` for any tied groups — the SDK derives which matches are needed and includes them. Worst case (all 16 players tied at 7-8 wins): ~120 match accounts. Likely CU-tight; document as a known constraint and add `finalize_round_robin_chunked` if 16-player tests fail.

### Modify: `instructions/cancel_tournament.rs`

No RR-specific changes. Existing cancel + refund flow works because RR uses the same vault and Participant PDAs.

### Constants

```rust
pub const MAX_RR_PARTICIPANTS: u16 = 16;
pub const RR_MATCHES_FOR_N_PLAYERS: fn(u16) -> u16 = |n| n * (n - 1) / 2;
```

### Errors

Add to `errors.rs`:
- `RoundRobinParticipantCapExceeded`
- `RoundRobinAdvancementInvalid` (defensive; should never fire)
- `RoundRobinScoreInconsistent` (winner doesn't match higher score, or score tied)
- `ScoreInconsistentWithWinner` (general; fires for any format when both scores non-zero and disagree with `winner`)
- `FormatMismatch` (when an ix expects one format but the tournament is another — e.g., calling `start_round` on a non-Swiss tournament)

### Events

`TournamentCreated` gains `format: u8`. `MatchReported` gains `score_a: u16` + `score_b: u16` (zero-default for unreported scores, preserves existing indexer parser behavior — old single-elim consumers see new fields as `0` and ignore). No new events for RR — `MatchReported` + `TournamentCompleted` are reused.

## SDK changes (Phase A)

### Files to modify

1. **`src/idl/`** — regenerate via `pnpm run sync-idl`.
2. **`src/types.ts`** — add `TournamentFormat` TS union (`'SingleElim' | 'DoubleElim' | 'Swiss' | 'RoundRobin'`); extend `Tournament` with `format`; extend `Match` with `scoreA`, `scoreB`; extend `Participant` with `wins`, `losses`, `pointsFor`, `pointsAgainst`.
3. **`src/pdas.ts`** — extend `findMatchPda(tournament, bracket, round, matchIndex)` (now takes `bracket`; default `0` for backwards-compat callsites).
4. **`src/methods/createTournament.ts`** — accept `format?: TournamentFormat` param (default `'SingleElim'`); validate `maxParticipants <= 16` when `format === 'RoundRobin'`.
5. **`src/methods/startTournament.ts`** — pre-compute RR pairings client-side via Circle method; pass match PDAs as `remaining_accounts` per chunk.
6. **`src/methods/reportResult.ts`** — accept `scoreA?: number`, `scoreB?: number` (default 0); pre-flight validate that `winner` agrees with the higher score when both non-zero; require non-zero scores when `tournament.format === 'RoundRobin'`. For RR, derive whether this is the final match and include the participant PDAs needed for `finalize_round_robin` via `remaining_accounts`.
7. **`src/methods/index.ts`** — export new helpers.

### New files

- **`src/formats/roundRobin.ts`** — pairing helpers:
  - `roundRobinPairings(participants: Pubkey[]): MatchPairing[]` (Circle method).
  - `roundRobinTotalMatches(n: number): number` (= `n * (n - 1) / 2`).
  - `computeRoundRobinStandings(participants, matches): Ranking[]` — used by SDK consumers for live standings (off-chain mirror of on-chain `finalize_round_robin`). Includes `differential` column derived from each participant's `pointsFor - pointsAgainst`.
- **`src/formats/index.ts`** — re-export per-format helpers; will gain `doubleElim.ts` (Phase B) and `swiss.ts` (Phase C).

### Dependencies (user-installed)

No new SDK deps for Phase A.

## Indexer changes (Phase A)

### `prisma/schema.prisma`

Extend `Tournament`:
```prisma
format    Int    @default(0)   // TournamentFormat enum
```

Extend `Participant`:
```prisma
wins            Int    @default(0)
losses          Int    @default(0)
pointsFor       Int    @default(0)   // cumulative score across all reported matches
pointsAgainst   Int    @default(0)
```

Extend `Match`:
```prisma
bracket   Int    @default(0)   // 0=Winners/only, 1=Losers, 2=Grand
scoreA    Int    @default(0)
scoreB    Int    @default(0)
```

No new tables for Phase A.

### `src/webhooks/helius-parser.service.ts`

- `TournamentCreated` now carries `format` — store on Tournament row.
- `MatchReported` now carries `bracket`, `score_a`, `score_b` — store on Match row; increment `Participant.wins` / `losses` accordingly; add scores to `pointsFor` / `pointsAgainst` for both participants.

### New endpoint

`GET /tournaments/:address/standings` — returns ranked participants for RR (and Swiss, when Phase C lands). Format-aware:
- RR: wins → H2H → differential (`pointsFor - pointsAgainst`) → join order.
- Swiss: wins → Buchholz → join order (Phase C).
- SingleElim/DoubleElim: returns `null` (these don't use standings; placements come from the bracket).

Response includes `differential` column for RR/Swiss (always present, but only used for sort in RR — Swiss surfaces it as a secondary display only).

### `src/reconciliation/reconciliation.service.ts`

Surface new fields when fetching Tournament/Participant accounts during drift recovery.

## Frontend changes (Phase A)

### Files to modify

1. **`types/tournament.ts`** — add `TournamentFormat` union; extend `Tournament` with `format`; extend `Match` with `scoreA`, `scoreB`; extend `Participant` with `wins`, `losses`, `pointsFor`, `pointsAgainst`.
2. **`lib/indexerToTournamentState.ts`** — map new fields.
3. **`features/tournament/create/CreateTournament.tsx`** — the `FormatPicker` already exists in the UI (per `[[project_v1_format_expansion]]`); wire its output through to the SDK call. Add 16-player cap warning when `format === 'RoundRobin'` is selected.
4. **`features/tournament/steps/ValidateState.ts`** — enforce RR cap.
5. **`features/tournament/view/BracketView.tsx`** — RR layout: render a grid of all-vs-all matches (rotation table), not a bracket tree.
6. **`features/tournament/view/ReportResultModal.tsx`** — add score inputs (two numeric fields, `scoreA` / `scoreB`). For RR, scores are required and validated client-side. For SE / DE / Swiss, scores are optional (placeholder "0" — leave blank to skip).

### New files

1. **`features/tournament/view/RoundRobinGrid.tsx`** — N×N grid showing every pairing. Cells show match result (`score_a-score_b`) or "Pending."
2. **`features/tournament/view/StandingsTable.tsx`** — used by RR + Swiss. Columns: rank, player, wins, losses, differential (RR) / Buchholz (Swiss).
3. **`hooks/useStandings.ts`** — TanStack Query against `GET /tournaments/:address/standings`. Polls every 10s when tournament is Active.

### Out of frontend scope for Phase A

- Swiss-specific UI (Phase C).
- DE bracket layout (Phase B).
- Per-match score input (out of plan scope — V1 uses winner-only).

---

# Phase B — Double Elimination

## The topology change

Single elim is a binary tree: each match feeds *one* parent. Double elim has two trees plus a grand final:
- **Winners bracket (WB):** identical to single elim — winners advance, losers drop to LB.
- **Losers bracket (LB):** receives losers from WB at staggered positions. Losers in LB are eliminated; LB winners advance.
- **Grand Final (GF):** WB champion vs LB champion. If WB champion wins, tournament over. If LB champion wins, **bracket resets** — one more match between them.

For `N = 2^k` players (V1 supports DE for power-of-2 fields only):
- WB has `N - 1` matches (binary-tree elimination).
- LB has `N - 1` matches (extended structure — see implementation note below).
- GF has 1 match (no reset) or 2 matches (with reset).

**Total: `2N - 1` (no reset) or `2N` (with reset).**

For 8 players: SE = 7 matches, DE = 15 or 16 matches.
For 16 players: SE = 15, DE = 31 or 32.
For 128 players: SE = 127, DE = 255 or 256.

`★ Insight ─────────────────────────────────────`
The LB structure used here (N-1 matches) is slightly larger than the Challonge minimal structure (N-2 matches). The extra match lives in the late LB rounds — concretely, the LB champion plays one more match than the bare minimum before qualifying for GF. The benefit: a more symmetric bracket where the LB run feels meaningful (the LB champion has fought through the same depth as the WB champion). The cost: 1 extra `MatchNode` PDA per tournament. For 128-player DE, that's 1 PDA out of ~256 — negligible rent.
`─────────────────────────────────────────────────`

`★ Insight ─────────────────────────────────────`
The doubling of match count is why DE has a CU budget risk at 128p that single-elim doesn't. `start_tournament` for 128p single elim takes ~19 chunks (7 inits/chunk × 19 = 133 ≥ 127); 128p DE would need ~37 chunks. The existing chunking pattern absorbs this — no design change, just more chunks per tournament. Worth flagging as a Tier-4 capacity test once Phase B lands (mirroring the MVP plan's 128p single-elim test).
`─────────────────────────────────────────────────`

## Loser drop mapping

The non-trivial part of DE on-chain is the **loser-advancement graph**: when player loses WB match (round R, index I), they drop to a specific LB position. The mapping is determinate but format-specific (most communities use "losers minor / losers major" alternating structure).

V1 uses the canonical mapping from Challonge / smash.gg:
- WB round 1 losers → LB round 1.
- WB round R>1 losers → LB round `2*(R-1)`, paired with LB-bracket survivors per a fixed permutation.
- WB final loser → LB final.
- LB final winner → GF.

The exact index mapping is a closed-form function of `(N, wb_round, wb_match_index)`. Implementing it as a Rust function on-chain: ~40 LOC. Same function lives in the SDK for client-side derivation.

## Program changes

### Modify: `state/tournament.rs`

```rust
pub grand_final_reset: bool,     // whether LB-champion wins force a reset match
```

Default `true` (standard DE behavior). Set at `create_tournament` time, immutable after.

### Modify: `state/match_node.rs`

Add a compact destination tuple:

```rust
pub winner_dest: MatchDest,       // where the winner of this match advances
pub loser_dest: MatchDest,        // where the loser of this match drops (DE only)
```

```rust
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, InitSpace, PartialEq, Eq)]
pub struct MatchDest {
    pub bracket: u8,              // 0/1/2 for WB/LB/GF; 0xFF = no destination (eliminated/championed)
    pub round: u8,
    pub match_index: u16,
    pub slot: u8,                 // 0 = player_a slot, 1 = player_b slot
}
```

Account-size cost: +5 bytes × 2 = +10 bytes per match. Slightly higher than the +8 estimated in the cross-cutting table; updated in critical-files index.

For non-DE formats, both fields are `bracket = 0xFF` (no destination — single-elim derives advancement from `(round + 1, match_index / 2)` arithmetically, but storing it explicitly is uniformly simpler and removes the format-aware advancement code path).

`★ Insight ─────────────────────────────────────`
Storing `winner_dest` / `loser_dest` explicitly per match is a deliberate trade: ~7 KB more per 128p tournament in exchange for collapsing four format-specific advancement functions into one shared `advance_winner(match_node, winner_pubkey, winner_dest)`. The alternative — keep `advance_winner` format-aware — means every new format adds a new advancement function. Explicit destinations make `report_result` data-driven, which is exactly the property that makes oracle-reported results (V1.2) drop in cleanly later.
`─────────────────────────────────────────────────`

### Modify: `instructions/create_tournament.rs`

Extend signature with `grand_final_reset: Option<bool>` (defaults to `true` for DE, ignored otherwise). Validation:
- `format == DoubleElim` requires `max_participants` to be a power of 2 (DE doesn't naturally accommodate odd counts; first-round byes are out of plan scope).
- `2 <= max_participants <= 128`.

Update `TournamentCreated` event to include `grand_final_reset`.

### Modify: `instructions/start_tournament.rs`

DE-specific init path:
1. Compute total matches: `2N - 1` (if `grand_final_reset == false`) or `2N` (if `true`).
2. Init WB matches (`N - 1` total) with `winner_dest` = next WB position (or GF for WB final) and `loser_dest` = corresponding LB position (or eliminated for WB-final loser → LB-final-adjacent slot).
3. Init LB matches (`N - 1` total — extended structure) with `winner_dest` = next LB position (or GF for LB final) and `loser_dest` = eliminated (`bracket = 0xFF`).
4. Init GF match 1 with `winner_dest` = (GF match 2 if reset is enabled, else eliminated/champion) and `loser_dest` = eliminated (LB champion is at minimum 2nd place).
5. Init GF match 2 (conditional on `grand_final_reset`) with `winner_dest` = eliminated/champion.

Chunked: same 7-inits/chunk pattern; ~37 chunks for 128p DE with reset.

### Modify: `instructions/report_result.rs`

Dispatch on format:
- `DoubleElim`:
  - Validate match Active, winner ∈ {a, b}.
  - Mark match Completed, set winner.
  - **Advance winner** to `match.winner_dest`.
  - **Advance loser** to `match.loser_dest` (only DE — for other formats, `loser_dest.bracket == 0xFF` and this step is a no-op).
  - Detect tournament-end:
    - If reported match is GF1 and WB champion won → distribute prizes (champion = GF1 winner, 2nd = LB champion = GF1 loser).
    - If reported match is GF1 and LB champion won → reset bracket: activate GF2; tournament stays Active.
    - If reported match is GF2 → distribute prizes (champion = GF2 winner, 2nd = GF2 loser).
- Other formats: unchanged path.

Loser-advancement logic:

```rust
fn advance_loser(
    tournament: &mut Tournament,
    match_node: &MatchNode,
    loser_pubkey: Pubkey,
    loser_dest_match: &mut MatchNode,
) -> Result<()> {
    if match_node.loser_dest.bracket == 0xFF {
        // No destination — eliminated. Nothing to do.
        return Ok(());
    }
    if match_node.loser_dest.slot == 0 {
        loser_dest_match.player_a = loser_pubkey;
    } else {
        loser_dest_match.player_b = loser_pubkey;
    }
    // If both slots now populated, transition match to Active.
    if loser_dest_match.player_a != Pubkey::default()
       && loser_dest_match.player_b != Pubkey::default() {
        loser_dest_match.status = MatchStatus::Active;
    }
    Ok(())
}
```

### Modify: `instructions/cancel_tournament.rs`

No DE-specific changes.

### Constants

```rust
pub const MATCH_DEST_NONE: u8 = 0xFF;   // sentinel in MatchDest.bracket
```

### Errors

Add:
- `DoubleElimRequiresPowerOfTwo`
- `MatchDestInvalid` (defensive; never fires)
- `GrandFinalResetInvalidState` (defensive; never fires)

### Events

No new events for DE. `MatchReported` is emitted for every DE match including GF; `TournamentCompleted`'s `placement_payouts` covers ranking.

## SDK changes (Phase B)

### Files to modify

1. **`src/types.ts`** — add `MatchDest`; extend `Tournament` with `grandFinalReset`; extend `Match` with `winnerDest`, `loserDest`.
2. **`src/pdas.ts`** — `findMatchPda` already takes `bracket` from Phase A.
3. **`src/methods/createTournament.ts`** — accept `grandFinalReset?: boolean` (default `true`); validate power-of-2 when DE.
4. **`src/methods/startTournament.ts`** — pre-compute DE match-init descriptors (WB, LB, GF) via `formats/doubleElim.ts`; pass via `remaining_accounts` per chunk.
5. **`src/methods/reportResult.ts`** — for DE, derive whether the reported match completes the tournament (with or without reset).

### New file

- **`src/formats/doubleElim.ts`** — bracket builder:
  - `doubleElimMatches(n: number, reset: boolean): MatchInitDescriptor[]` — emits all WB(N-1) / LB(N-1) / GF(1 or 2) matches with `winnerDest` / `loserDest` populated. Returns `2N-1` descriptors (no reset) or `2N` (with reset).
  - `loserDropPosition(wbRound: number, wbMatchIndex: number, n: number): MatchDest` — the WB→LB drop mapping for the extended-LB structure.
  - `nextDoubleElimMatch(match: Match, winner: Pubkey): MatchDest` — used for live-view bracket animation.

The drop mapping is closed-form; ~50 LOC (slightly more than Challonge-minimal because the extra LB round has its own drop rule). Lock the algorithm spec in `formats/doubleElim.ts` and reference it from the program-side init code.

### Dependencies (user-installed)

No new SDK deps for Phase B.

## Indexer changes (Phase B)

### `prisma/schema.prisma`

Extend `Tournament`:
```prisma
grandFinalReset    Boolean    @default(false)
```

Extend `Match`:
```prisma
winnerDestBracket   Int?
winnerDestRound     Int?
winnerDestMatch     Int?
winnerDestSlot      Int?
loserDestBracket    Int?
loserDestRound      Int?
loserDestMatch      Int?
loserDestSlot       Int?
```

(Flat columns are cheaper than a JSON blob for the indexer's join patterns.)

### `src/webhooks/helius-parser.service.ts`

- `TournamentCreated` now carries `grandFinalReset` — store on Tournament.
- `MatchReported` for DE: the indexer's read path uses `winnerDest` / `loserDest` to compute UI bracket position. Surface these via the API.

### New endpoint

`GET /tournaments/:address/bracket` already exists (or rolls up from `getMatches`). Extend response to include `format` so the frontend selects the right `BracketView` variant.

## Frontend changes (Phase B)

### Files to modify

1. **`types/tournament.ts`** — add `MatchDest`; extend `Tournament` with `grandFinalReset`; extend `Match`.
2. **`lib/indexerToTournamentState.ts`** — map new fields.
3. **`features/tournament/view/BracketView.tsx`** — dispatcher on `tournament.format`:
   - `SingleElim` → existing tree layout.
   - `DoubleElim` → new `DoubleElimBracketView`.
   - `RoundRobin` → `RoundRobinGrid` (Phase A).
   - `Swiss` → `SwissRoundsView` (Phase C).
4. **`features/tournament/create/CreateTournament.tsx`** — show `grandFinalReset` checkbox when DE is selected; validate power-of-2 participant count.

### New files

1. **`features/tournament/view/DoubleElimBracketView.tsx`** — side-by-side WB and LB trees with the GF panel below. Animated edge from each WB loss to the LB drop position. Reuses existing `MatchCard` from single-elim.
2. **`components/BracketResetBadge.tsx`** — small badge shown on GF when LB champion forces a reset.

---

# Phase C — Swiss

**Prerequisite:** V1 player-reported plan's VRF surface (`request_seed`, `reveal_seed`, `tournament.seed_hash`, `seed_revealed`) must ship first. Swiss uses VRF for round-1 pairing entropy — organizer-submitted seeds are rejected as a design choice (Swiss pairings are deterministic from the seed, so an organizer-controlled seed is equivalent to an organizer-controlled bracket, which defeats the format's competitive fairness).

If V1's VRF section is delayed for any reason, Phase C is held back; Phases A and B can ship independently. The dependency is one-way — VRF doesn't need Swiss.

## What Swiss is

Swiss is the tournament format used by chess, Magic, and most trading-card games. Every player plays a **fixed number of rounds** (configurable; typically 3-9 depending on field size). No elimination. Each round, players are paired against opponents with similar records. Same opponent pairings are not allowed within a tournament.

After the final round, players are ranked by:
1. Wins (primary).
2. **Buchholz score** (sum of opponents' wins) — measures strength of schedule.
3. Join order (final fallback).

The top `K` placements receive prizes per the chosen `payout_preset`.

## What makes Swiss harder than RR/DE on-chain

- **Per-round pairing is not pre-computable.** Round 2's pairings depend on round 1's results. So `start_tournament` cannot pre-init all matches — only round 1.
- **Opponent history must be tracked on-chain** to enforce the no-repeat rule. Naively this is `O(rounds)` Pubkeys per Participant.
- **Buchholz computation at finalization** requires reading every Participant + every match — high CU + high `remaining_accounts` count.
- **Pairing algorithm has multiple correct implementations** (Dutch / Monrad / accelerated Swiss). V1 uses the simplest: rank players by current wins, descending; pair top half with bottom half within each win-group; resolve no-repeat conflicts greedily.

V1 implements pairing **client-side** (in the SDK), with the program **validating** the proposed pairing meets the constraints. Same pattern as V1's VRF-bracket: client computes, program validates. Keeps on-chain CU low.

## Program changes

### Modify: `state/tournament.rs`

```rust
pub rounds: u8,                    // configured at create time; 3-9
pub current_round: u8,             // 0 before first start_round; increments with each round
pub round_active: bool,            // true while a round is in progress
```

Account-size cost: 1 + 1 + 1 = 3 bytes. The `pairing_seed` is reused from V1's `tournament.seed_hash` — no new field.

**Pairing entropy comes from V1's VRF.** Swiss tournaments inherit V1's `settlement_mode != OrganizerOnly` gate on `start_tournament`, which forces `request_seed` + `reveal_seed` before pairing begins. The revealed `tournament.seed_hash: [u8; 32]` is used as the seed input for round-1 pairing (subsequent rounds are deterministic from results, so no further entropy needed). This means an organizer cannot rig round-1 pairings by submitting biased entropy — the validator-unmanipulable VRF reveal is the only entropy source.

### Modify: `state/participant.rs`

Already has `wins`, `losses` from Phase A. Add:

```rust
pub byes_received: u8,             // counted toward wins; tracked separately for tiebreaker
pub current_round_match: u32,      // packed (bracket << 24) | (round << 16) | match_index; 0 if not yet paired this round
```

The `current_round_match` is a denormalized pointer set when a round starts; it lets the frontend show "your next match" without iterating all matches.

### New account: `SwissPairingHistory`

```rust
#[account(InitSpace)]
pub struct SwissPairingHistory {
    pub tournament: Pubkey,
    pub player: Pubkey,
    pub opponents: [Pubkey; MAX_SWISS_ROUNDS],   // MAX_SWISS_ROUNDS = 9
    pub opponent_count: u8,                       // how many of the array slots are populated
    pub bump: u8,
}
```

PDA seed: `[b"swiss_history", tournament, player]`. Created lazily on first pairing (i.e., at `start_round(0)` for every paired player; bye-receivers don't need it created until round 2).

Per-player size: 32 + 32 + 9*32 + 1 + 1 = 354 bytes. For 64-player Swiss: ~22 KB total. Reasonable.

`★ Insight ─────────────────────────────────────`
This is the one place in the formats plan where account-size pressure is real. A separate `SwissPairingHistory` PDA per (tournament, player) means the rent overhead scales with players × Swiss-mode tournaments. For mainnet, this should be considered for compression — a single tournament-level `PairingHistory` PDA with bitmaps per round, indexed by participant `seed_index`, would shrink it ~5×. For V1 on devnet, the per-player PDA is clearer and the rent doesn't matter.
`─────────────────────────────────────────────────`

### Modify: `instructions/create_tournament.rs`

Extend signature with `rounds: Option<u8>` (default `ceil(log2(N))` — chess-like). Validation:
- `format == Swiss` requires `rounds >= MIN_SWISS_ROUNDS && rounds <= MAX_SWISS_ROUNDS` → `SwissRoundsOutOfRange`.
- `format == Swiss` requires `settlement_mode != OrganizerOnly` (V1 SettlementMode enum) → `SwissRequiresVrfSettlement`. This is the V1-VRF dependency hook: without a non-OrganizerOnly settlement mode, V1's `start_tournament` skips the VRF gate, leaving Swiss with no entropy source.

No `pairing_seed` param — the seed is V1's `tournament.seed_hash`.

### Modify: `instructions/start_tournament.rs`

For Swiss:
- Validate V1's VRF gate fires: `seed_revealed == true` → `SeedNotRevealed` (this error is V1's).
- Transition status to `Active`.
- Do **not** pre-init any matches.
- Set `tournament.current_round = 0`, `tournament.round_active = false`.

`start_tournament` only validates inputs and flips status; pairing happens in `start_round`, which reads `tournament.seed_hash` as the entropy source.

### New ix: `instructions/start_round.rs`

Organizer-signed. Inputs: `round_pairings: Vec<MatchPairing>` (passed via instruction data or `remaining_accounts`).

Handler:
1. Require `tournament.format == Swiss` and `tournament.status == Active` and `tournament.round_active == false`.
2. Require `tournament.current_round < tournament.rounds`.
3. **Validate proposed pairings are consistent with the deterministic algorithm.** The program re-derives the expected pairings from `(participants, histories, tournament.seed_hash, current_round)` and checks the submitted `round_pairings` matches. Mismatch → `PairingNotDeterministic`. This is the same "client computes, program validates" pattern V1 uses for VRF-derived brackets.
4. For each pairing `(a, b)`:
   - Validate `a` and `b` are registered participants.
   - Validate `(a, b)` not already in either's `SwissPairingHistory.opponents` (redundant with step 3 if the algorithm is correct, but defensive).
   - Init `MatchNode` PDA at `[b"match", tournament, 0, round, match_index]`.
   - Append `b` to `a`'s history; append `a` to `b`'s history.
5. If odd participant count, exactly one player gets a bye:
   - Bye recipient is the lowest-ranked player who hasn't received a bye yet (`byes_received == 0`), deterministic from the standings + seed.
   - Increment `byes_received` and `wins` for the bye recipient (bye = automatic win, scored as `1-0` for differential purposes).
   - No `MatchNode` created for the bye.
6. Set `tournament.round_active = true`, increment `tournament.current_round`.
7. Emit `RoundStarted { tournament, round, pairing_count, bye_recipient }`.

Chunked: same 7-init/chunk pattern; ~10 chunks for 64-player Swiss.

`★ Insight ─────────────────────────────────────`
Step 3 — program re-derives pairings — is what locks the determinism. Without it, an organizer could submit any pairing they liked under the cover of "Swiss algorithms have multiple valid implementations." With it, the only freedom the organizer has is *when* to start the round, not *who* plays whom. The VRF seed eliminates organizer entropy control; the deterministic algorithm eliminates organizer pairing control. Together they make Swiss-on-chain genuinely trust-minimized.
`─────────────────────────────────────────────────`

### Modify: `instructions/report_result.rs`

For Swiss:
- Validate match Active, winner ∈ {a, b}.
- Mark match Completed, set winner.
- Increment `winner.wins`, `loser.losses`.
- If `tournament.matches_reported_in_current_round == matches_in_current_round`:
  - Set `tournament.round_active = false`.
  - Emit `RoundCompleted { tournament, round }`.
  - If `current_round == tournament.rounds`, call `finalize_swiss(...)`.
- **No advancement** — Swiss matches don't feed into others.

### New ix: `instructions/finalize_swiss.rs`

Called inline from `report_result` when the last match of the last round is reported. Organizer-signed via parent.

Handler:
1. Iterate all Participant PDAs + all SwissPairingHistory PDAs (via `remaining_accounts`).
2. Compute Buchholz: for each player, sum `opponents[i].wins` across all opponents they've played.
3. Sort by `(wins DESC, buchholz DESC, join_order ASC)`.
4. Map ranks to `placement_payouts` per the chosen preset.
5. Distribute prizes + 3.5% protocol fee.
6. Set `tournament.champion`, `tournament.status = Completed`.
7. Emit `TournamentCompleted` with `placement_payouts`.

CU pressure: reading every Participant + every SwissPairingHistory for a 64-player Swiss means ~128 accounts. Likely at the CU ceiling for a single ix. If 64-player tests fail, add `finalize_swiss_chunked` (compute partial Buchholz per chunk, store intermediate ranking on a new `SwissFinalization` PDA, finalize at the end).

### Constants

```rust
pub const MAX_SWISS_ROUNDS: u8 = 9;
pub const MIN_SWISS_ROUNDS: u8 = 3;
```

### Errors

Add:
- `SwissRoundsOutOfRange`
- `SwissRequiresVrfSettlement` (Swiss requires V1's `settlement_mode != OrganizerOnly` so VRF gate fires)
- `PairingNotDeterministic` (submitted pairing diverges from re-derivation)
- `PairingHasRepeatMatch`
- `PairingPlayerNotInTournament`
- `RoundAlreadyActive`
- `RoundNotComplete`
- `SwissUsesStartRound` (when `start_tournament` is called on Swiss with the wrong shape)
- `ByeRecipientInvalid`

(V1's `SeedNotRevealed` is reused, not redefined.)

### Events

```rust
#[event] pub struct RoundStarted {
    pub tournament: Pubkey,
    pub round: u8,
    pub pairing_count: u16,
    pub bye_recipient: Option<Pubkey>,
}

#[event] pub struct RoundCompleted {
    pub tournament: Pubkey,
    pub round: u8,
}
```

## SDK changes (Phase C)

### Files to modify

1. **`src/types.ts`** — add `SwissPairingHistory`, `MatchPairing`; extend `Tournament` with `rounds`, `currentRound`, `roundActive` (V1's `seedHash` / `seedRevealed` are already typed); extend `Participant` with `byesReceived`, `currentRoundMatch`.
2. **`src/pdas.ts`** — add `findSwissPairingHistoryPda(tournament, player)`.
3. **`src/methods/createTournament.ts`** — accept `rounds`, `pairingSeed`; validate.
4. **`src/methods/startTournament.ts`** — no match init for Swiss; just flips status.
5. **`src/methods/reportResult.ts`** — for Swiss, derive whether the reported match completes the round/tournament.

### New files

- **`src/methods/startRound.ts`** — organizer-side; takes `round` number, calls `formats/swiss.ts:computePairings`, submits with `remaining_accounts`.
- **`src/formats/swiss.ts`**:
  - `swissDefaultRounds(n: number): number` — `Math.ceil(log2(n))`.
  - `computePairings(participants: ParticipantState[], histories: SwissPairingHistory[], seedHash: Uint8Array, round: number): MatchPairing[]` — implements the greedy same-record pairing with no-repeat constraint, seeded by V1's VRF-revealed `tournament.seed_hash`.
  - `computeBuchholz(participants, histories): Map<Pubkey, number>`.
  - `computeSwissStandings(participants, histories): Ranking[]`.

The pairing algorithm must match the on-chain re-derivation byte-for-byte — any divergence triggers `PairingNotDeterministic` on `start_round`. Lock the algorithm spec in `src/formats/swiss.ts` and reference it from the program-side validation.

### Dependencies (user-installed)

No new SDK deps for Phase C.

## Indexer changes (Phase C)

### `prisma/schema.prisma`

Extend `Tournament`:
```prisma
rounds            Int?
currentRound      Int    @default(0)
roundActive       Boolean    @default(false)
```
(V1's `seedHash` / `seedRevealed` columns are reused — no separate `pairingSeed` field.)

Extend `Participant`:
```prisma
byesReceived      Int    @default(0)
```

New table:
```prisma
model SwissPairingHistory {
  tournamentAddress  String
  player             String
  opponents          String[]      // array of pubkeys
  byes               Int    @default(0)
  @@id([tournamentAddress, player])
  @@index([tournamentAddress])
}
```

### `src/webhooks/helius-parser.service.ts`

Handlers for new events:
- `RoundStarted` → upsert SwissPairingHistory rows for each paired player; increment Participant.byesReceived for bye recipient.
- `RoundCompleted` → mark Tournament.roundActive = false in DB.

### `src/reconciliation/reconciliation.service.ts`

Surface `rounds`, `currentRound`, `roundActive` from Tournament; surface `byesReceived` from Participant.

### Standings endpoint extension

`GET /tournaments/:address/standings` now returns Swiss-aware ranking when `format == Swiss`. Buchholz computed server-side from cached SwissPairingHistory + Participant tables.

## Frontend changes (Phase C)

### Files to modify

1. **`types/tournament.ts`** — extend.
2. **`lib/indexerToTournamentState.ts`** — map.
3. **`features/tournament/create/CreateTournament.tsx`** — when Swiss is selected: show `rounds` slider (3-9, default `ceil(log2(N))`); show an info banner ("Swiss uses VRF for pairing — your tournament will require a VRF commit/reveal before round 1 starts" — reuse V1's `useVrfStatus` for the actual reveal flow). No organizer-side seed input.
4. **`features/tournament/view/BracketView.tsx`** — dispatch to `SwissRoundsView`.
5. **`features/tournament/view/TournamentSidebar.tsx`** — for Swiss, the "Start" button gates on V1's VRF status (reuse V1's `useVrfStatus` hook) and the per-round flow:
   - Before VRF requested: "Request VRF" (organizer) → V1's `request_seed`.
   - VRF committed but not revealed: "Awaiting VRF reveal…" with V1's manual "Reveal now" fallback.
   - VRF revealed, before round 1: "Start Round 1" → `startTournament` then `startRound(0)`.
   - Mid-tournament: "Start Round N+1" (visible when round N's last match reported).
6. **`features/tournament/view/StandingsTable.tsx`** (from Phase A) — gains Buchholz column for Swiss.

### New files

1. **`features/tournament/view/SwissRoundsView.tsx`** — tab per round; each tab shows that round's matches and any bye. The current round is the default-selected tab.
2. **`features/tournament/view/StartRoundModal.tsx`** — organizer-only; previews the SDK-computed pairings before committing, with a "Re-roll seed" affordance if the organizer wants different pairings (Swiss is deterministic from `(participants, histories, seed)`, so re-rolling means changing the seed for the next round only — implementation detail: store per-round seeds on Tournament as a `Vec<[u8; 32]>` if this becomes desired; out of plan scope for V1).
3. **`hooks/useNextPairings.ts`** — TanStack Query that returns the next round's proposed pairings by calling the SDK locally with current standings.

### Out of frontend scope for Phase C

- Per-round seed override (re-roll between rounds).
- Acceleration / pairing weights (Buchholz-aware initial pairings instead of join-order).
- Top-cut bracket after Swiss rounds (a hybrid format where Swiss seeds a single-elim final 8).

---

## Verification (end-to-end devnet smoke)

### Phase A — Round Robin

1. **Program tests** (`bracket-chain-programs/tests/`):
   - Create RR tournament with `max_participants = 16`, 16 players join → `start_tournament` inits 120 matches.
   - Each match reported with `(score_a, score_b)` → wins/losses + `points_for` / `points_against` accumulate on Participant accounts.
   - Score-inconsistent report: `winner = a` but `score_a < score_b` → `ScoreInconsistentWithWinner`.
   - RR-only: report with `score_a == score_b` → `RoundRobinScoreInconsistent` (RR matches cannot tie).
   - Final match reported → `finalize_round_robin` distributes prizes by wins → H2H → differential → join order.
   - Try to create RR with `max_participants = 17` → `RoundRobinParticipantCapExceeded`.
   - All-tied edge case: 4-player RR where everyone has equal wins + differential → resolved via join order; verify deterministic ranking.
   - Differential decides: 4-player RR where two players tie on wins + H2H but one has +5 differential and the other -5 → higher differential ranks higher.
2. **Indexer integration test**: spin up against devnet, verify `Participant.wins` / `losses` accumulate; verify `GET /tournaments/:address/standings` returns correct ranking.
3. **Frontend smoke**: create 4-player RR from the UI, complete all 6 matches, verify `RoundRobinGrid` updates live and `StandingsTable` shows correct order.

### Phase B — Double Elimination

1. **Program tests**:
   - 8-player DE (`reset = true`) inits exactly `2N = 16` MatchNodes; with `reset = false` inits `2N - 1 = 15`.
   - 8-player DE, walk through full bracket with no reset → champion = WB winner (15 matches played).
   - 8-player DE, force LB champion to win GF1 → GF2 activates → final winner = GF2 victor (16 matches played).
   - 16-player DE with reset disabled → GF1 winner is champion regardless (31 matches played).
   - Invalid: 7-player DE → `DoubleElimRequiresPowerOfTwo`.
   - Loser-drop mapping: verify a WB-R1 loser drops to LB-R1 at the expected slot, and a WB-final loser drops to the LB pre-final (extended LB structure).
2. **Indexer test**: verify `Match.bracket` is correctly populated (0/1/2) and `winnerDest` / `loserDest` are surfaced via the bracket API.
3. **Frontend smoke**: create 8-player DE, complete the full bracket, verify side-by-side WB/LB rendering and animated drop edges.

### Phase C — Swiss

1. **Program tests**:
   - 8-player Swiss with 3 rounds: V1's `request_seed` / `reveal_seed` → `start_tournament` → each round's `start_round` succeeds with valid pairings.
   - VRF gate: `start_tournament` without revealed seed → V1's `SeedNotRevealed`.
   - Swiss + `settlement_mode = OrganizerOnly` at create → `SwissRequiresVrfSettlement`.
   - Non-deterministic pairings: submit a pairing that differs from the re-derived expected pairing → `PairingNotDeterministic`.
   - No-repeat: attempt to pair players who have already played → `PairingHasRepeatMatch` (or `PairingNotDeterministic` first, since the deterministic algorithm avoids repeats).
   - Bye: 7-player Swiss; verify the lowest-ranked player gets the bye and their `wins` + `byes_received` both increment.
   - Round before previous complete: `RoundNotComplete`.
   - Final round complete: `finalize_swiss` computes Buchholz correctly and distributes prizes.
2. **SDK pairing-determinism test**: same `(participants, histories, seedHash, round)` → same `computePairings` output across two SDK invocations and across SDK / program (the byte-for-byte match is enforced on-chain).
3. **Indexer test**: `RoundStarted` / `RoundCompleted` events populate the DB; `SwissPairingHistory` rows update; standings API returns Buchholz column. V1's `SeedRequested` / `SeedRevealed` events populate the Tournament's VRF state.
4. **Frontend smoke**: create 8-player Swiss with 3 rounds, walk through VRF request → reveal → start round 1 → complete each round via the modal, verify standings update and Buchholz column populates after round 1.

### Capacity tests (Tier-4 polish, post-Phase-C)

- 128-player DE init: 36+ chunks; verify no CU overflows.
- 64-player Swiss finalize: ~128 accounts in `remaining_accounts`; verify CU survives or `finalize_swiss_chunked` is needed.
- 16-player RR finalize with all-ties: ~136 accounts; same CU question.

---

## Open questions to resolve before kickoff

1. **PDA seed change → fresh program ID vs migration ix.** Recommend fresh program ID, mirroring V1.1's decision. Existing single-elim devnet tournaments become unreachable; acceptable since the MVP is not on mainnet and the V1.1 plan already abandons the current program ID. Confirm OK to bundle this with V1.1's redeploy if both are in flight.
2. **Per-tournament rounds for Swiss.** Default to `ceil(log2(N))` (chess-like)? Or fixed 5 rounds? Recommend `ceil(log2(N))` as the SDK default with organizer override.
3. **Buchholz vs Sonneborn-Berger for Swiss tiebreakers.** Buchholz is simpler and widely understood; SB is more accurate but doubles the complexity. Recommend Buchholz for V1.
4. **DE grand-final reset default.** Most communities default to `reset = true` (LB run is rewarded with a real chance). Recommend default `true`.
5. **RR draw policy.** Spec says RR matches can't tie (`score_a != score_b` required). Some sports (Dota 2 Best-of-1) can end tied — confirm RR is best-of-N (where N is odd) so ties are impossible, or relax to allow `0.5-0.5` outcomes. Default plan: ties disallowed for RR; format requires odd best-of-N matches.
6. **Swiss odd-N bye policy.** "Lowest-ranked who hasn't byed yet" is the standard rule. If everyone in the field has byed, repeat with secondary tiebreaker. Edge case for very long Swiss tournaments; document as a known constraint.
7. **`SwissPairingHistory` per-player vs tournament-level.** Per-player is clearer; tournament-level is ~5× smaller. Recommend per-player for V1 (devnet rent doesn't matter); revisit before mainnet.
8. **Pairing computation: client-side vs program-side.** Plan chooses client-side computation with program-side validation via re-derivation. Trust profile is equivalent to full on-chain computation (any divergence rejected), at ~half the CU cost. Confirm OK.
9. **V1 / formats sequencing.** Phase C blocks on V1's VRF surface. If V1's player-reported plan is large enough that VRF lands late, Phase C waits. Phases A and B can ship in parallel with V1 development. Confirm acceptable timeline.

---

## Explicitly out of scope (V1+ follow-up plans)

- **Top-cut after Swiss.** Common hybrid format (Swiss rounds → single-elim final 8). Treatable as creating a SingleElim child tournament seeded from Swiss standings; separate plan.
- **Third-place playoff in DE.** Current DE plan: 3rd = LB-semifinal loser. Some communities run an explicit 3rd-place match. Out of scope.
- **Bye-balancing across multiple Swiss tournaments / leagues.** Cross-tournament state; out of scope.
- **Pairing acceleration (chess "accelerated Swiss").** First round pairs top vs upper-mid instead of top vs bottom. Out of scope.
- **Round-time enforcement (Swiss/RR time controls).** No on-chain timer; organizer manually closes rounds. Out of scope.
- **Dutch / Monrad pairing systems.** V1 uses greedy same-record-with-no-repeat. Alternative pairing systems are separate plans.
- **Format conversion mid-tournament.** Not supported (and not desired).

---

## Critical files (quick reference)

**Program — Phase A (RR):**
- `bracket-chain-programs/programs/bracket-chain/src/state/format.rs` — new (`TournamentFormat` enum)
- `bracket-chain-programs/programs/bracket-chain/src/state/tournament.rs` — add `format`
- `bracket-chain-programs/programs/bracket-chain/src/state/match_node.rs` — add `bracket`, `score_a`, `score_b`
- `bracket-chain-programs/programs/bracket-chain/src/state/participant.rs` — add `wins`, `losses`, `points_for`, `points_against`
- `bracket-chain-programs/programs/bracket-chain/src/instructions/create_tournament.rs` — extend params, validate RR cap
- `bracket-chain-programs/programs/bracket-chain/src/instructions/start_tournament.rs` — dispatch on format; RR init path
- `bracket-chain-programs/programs/bracket-chain/src/instructions/report_result.rs` — dispatch on format; RR finalize trigger
- `bracket-chain-programs/programs/bracket-chain/src/instructions/finalize_round_robin.rs` — new
- `bracket-chain-programs/programs/bracket-chain/src/instructions/migrate_v1_formats_tournament.rs` — new
- `bracket-chain-programs/programs/bracket-chain/src/constants.rs` — `MAX_RR_PARTICIPANTS`
- `bracket-chain-programs/programs/bracket-chain/src/errors.rs` — `RoundRobinParticipantCapExceeded`, `RoundRobinScoreInconsistent`, `ScoreInconsistentWithWinner`, `FormatMismatch`

**Program — Phase B (DE):**
- `state/tournament.rs` — add `grand_final_reset`
- `state/match_node.rs` — add `winner_dest`, `loser_dest` (uses new `MatchDest` struct in same file)
- `instructions/start_tournament.rs` — extend with DE init path (WB + LB + GF)
- `instructions/report_result.rs` — DE advancement (winner + loser destinations); detect GF1 reset
- `instructions/create_tournament.rs` — validate power-of-2 for DE; capture `grand_final_reset`
- `errors.rs` — `DoubleElimRequiresPowerOfTwo`, `MatchDestInvalid`, `GrandFinalResetInvalidState`
- `constants.rs` — `MATCH_DEST_NONE`

**Program — Phase C (Swiss):** *(blocked on V1 VRF — see Phase C preamble)*
- `state/tournament.rs` — add `rounds`, `current_round`, `round_active` (V1's `seed_hash` / `seed_revealed` reused for entropy)
- `state/participant.rs` — add `byes_received`, `current_round_match`
- `state/swiss_pairing_history.rs` — new account
- `instructions/start_tournament.rs` — Swiss path (no pre-init); validate V1's VRF gate
- `instructions/start_round.rs` — new; reads `tournament.seed_hash`; validates submitted pairings via re-derivation
- `instructions/report_result.rs` — Swiss path (no advancement; round-end detection)
- `instructions/finalize_swiss.rs` — new
- `events.rs` — `RoundStarted`, `RoundCompleted`
- `errors.rs` — 9 Swiss-specific variants (incl. `SwissRequiresVrfSettlement`, `PairingNotDeterministic`)
- `constants.rs` — `MAX_SWISS_ROUNDS`, `MIN_SWISS_ROUNDS`

**SDK:**
- `bracket-chain-sdk/src/types.ts`, `pdas.ts`, `errors.ts`, `index.ts` — extensions across all three phases
- `bracket-chain-sdk/src/formats/index.ts`, `roundRobin.ts` (Phase A), `doubleElim.ts` (Phase B), `swiss.ts` (Phase C) — new
- `bracket-chain-sdk/src/methods/createTournament.ts`, `startTournament.ts`, `reportResult.ts` — extended for format dispatch
- `bracket-chain-sdk/src/methods/startRound.ts` — new (Phase C)
- `bracket-chain-sdk/scripts/sync-idl.mjs` — run after each program rebuild

**Indexer:**
- `bracket-chain-indexer/prisma/schema.prisma` — extend `Tournament`, `Match`, `Participant`; new `SwissPairingHistory` table
- `bracket-chain-indexer/src/webhooks/helius-parser.service.ts` — handle `RoundStarted` / `RoundCompleted`; extend `TournamentCreated` / `MatchReported` parsers
- `bracket-chain-indexer/src/tournaments/tournaments.controller.ts` — new `GET /tournaments/:address/standings` endpoint
- `bracket-chain-indexer/src/reconciliation/reconciliation.service.ts` — surface new fields

**Frontend:**
- `BracketChain-Frontend/types/tournament.ts` — extend across all three phases
- `BracketChain-Frontend/lib/indexerToTournamentState.ts` — map new fields
- `BracketChain-Frontend/features/tournament/create/CreateTournament.tsx` — wire format picker (already exists in UI) through to SDK
- `BracketChain-Frontend/features/tournament/view/BracketView.tsx` — dispatcher on `tournament.format`
- `BracketChain-Frontend/features/tournament/view/RoundRobinGrid.tsx`, `DoubleElimBracketView.tsx`, `SwissRoundsView.tsx`, `StandingsTable.tsx`, `StartRoundModal.tsx` — new
- `BracketChain-Frontend/hooks/useStandings.ts`, `useNextPairings.ts` — new
- `BracketChain-Frontend/components/BracketResetBadge.tsx` — new

**Docs:**
- `bracketchain-main/README.md` — add format-expansion section to V1 setup.
- `bracketchain-mvp-plan.md` — add Phase reference to this plan in the "V1+ candidates" section.
- `bracketchain-v1.1-plan.md` — if both plans are in flight, note that they share the fresh-program-ID redeploy.
- `bracketchain-v1-player-reported-plan.md` — note that Phase C (Swiss) of the formats plan **depends on** this plan's VRF surface (`request_seed` / `reveal_seed` / `tournament.seed_hash`). Phases A (RR) and B (DE) ship independently and do not depend on V1.
