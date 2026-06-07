# CU Budget Baseline

> **⚠️ Variant B note (2026-06-05, R13 ratified):** the organizer deposit now
> stays in the vault on completion (no refund CPI in the final-match path), so
> every `report_result FINAL` row below is **stale on the high side** — the
> Variant-A measurement included a vault→organizer transfer CPI that no longer
> exists (teammate's Variant-B measurement on the MVP architecture saw the Deep
> 128p final drop ~80.5k → ~35.5k CU). Re-run `capacity-128p-deep` after the
> Stage F redeploy and refresh the FINAL rows; non-final rows are unaffected.

> **Phase 0 Section 3.4 deliverable.** Captures `meta.computeUnitsConsumed`
> per instruction × preset on the current MVP program
> (`AuXJKpuZtkegs2ZSgopgckhN7Ev8bUz4zBc238LD2F1` on devnet).
>
> Phase 1 redeploy (step 88) re-runs the `capacity-128p-deep` test and the
> CU log lines emitted by `bracket-chain.ts` WTA/Standard/Cancel tests, then
> diffs each row against this file. Treat this as a **regression contract**:
> growth bigger than the band noted in the "Phase 1 headroom" column is a
> review-blocker.
>
> Measurement method: each tx's signature → `connection.getTransaction(sig,
> { maxSupportedTransactionVersion: 0, commitment: "confirmed" })` →
> `meta.computeUnitsConsumed`. See `tests/utils.ts:measureCu`.

## Per-instruction baseline (Anchor 0.32.1, single-tx legacy)

Single-tx CU ceiling: **1_400_000** (Solana's per-tx hard cap; the
`ComputeBudgetProgram.setComputeUnitLimit` we attach to `start_tournament`
chunks sets the cap explicitly because the default 200k is too tight for
the chunked `init` pattern at 128p).

| # | Instruction | Variant / preset | Baseline CU | Phase 1 headroom |
|---|---|---|---:|---:|
| 1 | `initialize_protocol` | — (one-shot per cluster) | n/a (not on hot path) | — |
| 2 | `create_tournament` | Deep, `organizer_deposit > 0` | 35,074 | up to ~70k after V1.1 SAS schema |
| 3 | `join_tournament` | first / mid / last (128p) | 22,708 / 21,208 / 22,708 | up to ~50k after V1.1 identity-hash + attestation read |
| 4 | `start_tournament` | full chunk (7 matches) | 38,873 | up to ~80k after V1 proposal-envelope realloc |
| 4 | `start_tournament` | trailing partial chunk (5 matches) | 11,313 | up to ~30k |
| 5 | `report_result` | non-final, any round | 15,198 | up to ~40k after V1 dispute-window writes |
| 5 | `report_result` | FINAL, WTA, 8p (1 placement) | 28,762 | up to ~60k |
| 5 | `report_result` | FINAL, Standard, 8p (3 placements) | 43,417 | up to ~80k |
| 5 | `report_result` | FINAL, Deep, 128p (7 placements + organizer-deposit refund per Variant A) | 80,539 | up to ~150k (worst-case path) |
| 6 | `cancel_tournament` | 4 participants in one chunk, no deposit | 39,613 | up to ~80k (partial-cancel V1) |

## Final-match payout — placement loop cost

Linear in `placement_count`. Observed CU/placement:

| placements | total final CU | implied per-placement Δ |
|---:|---:|---:|
| 1 (WTA, 8p) | 28,762 | — (baseline) |
| 3 (Standard, 8p) | 43,417 | (43417 − 28762) / 2 ≈ **7,330 CU per extra placement** |
| 7 (Deep, 128p) + organizer-deposit refund | 80,539 | (80539 − 28762) / 6 + refund-CPI ≈ same magnitude |

Per-placement cost is dominated by `validate_token_account` (≈3k) + the
SPL `token::transfer` CPI (≈3–4k) + the `PlacementPayout` Vec push (≈0).
Variant A's organizer-deposit refund adds one extra `token::transfer` CPI
(~7k), accounting for the gap between Deep-128p's 80,539 and the linear
extrapolation from Std-8p (which would predict ≈73k).

## Headroom analysis vs Phase 1 growth

Phase 1 redeploy adds:

- **MatchNode +83 bytes** — proposal envelope (7 fields).
- **Participant +74 bytes** — `identity_hash`, `identity_attestation`,
  `wins`, `losses`, `points_for`, `points_against`.
- **`event_version: u8`** prepended to every `#[event]` (≈0 CU).
- **VRF reveal CPI** — only on the new `reveal_seed` ix, not in
  `start_tournament`.

Even the heaviest current path (`report_result` FINAL Deep 128p) sits at
**5.8 %** of the 1.4M ceiling. The proposal-envelope deserialize cost
(~32 bytes per match × constant overhead) is bounded at ≤ 2k CU. There
is no row where Phase 1 growth threatens the cap on a single tx.

## Re-running this baseline

```bash
anchor test --provider.cluster localnet
```

CU samples are printed inline by:

- `tests/bracket-chain.ts` — WTA / Standard final-match + Cancel rows
- `tests/capacity-128p-deep.test.ts` — every other row (creates a Deep
  tournament with `organizer_deposit > 0`, joins 128 players, runs all
  127 matches, samples 11 representative ix points)

For Phase 1 redeploy regression: diff this file against a fresh run on
the redeployed program and fail the ceremony if any row grows by more
than its stated headroom.
