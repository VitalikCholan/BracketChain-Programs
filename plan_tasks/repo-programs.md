# BracketChain-Programs — план робіт

> Anchor 0.32.1 program. Path: `/home/min/git-workspace/BracketChain-Programs/`.
> Поточний стан: MVP (6 ix) + Phase 2.5 (organizer deposit, multi-token, name≤32) + Phase 2.6 (`name` в `TournamentCreated`). Devnet program ID `AuXJKpuZtkegs2ZSgopgckhN7Ev8bUz4zBc238LD2F1`.

---

## Phase 0 — pre-Phase-1 hygiene

### Section 2 — Codama codegen pipeline (Stage 1)

- [ ] **Крок 20.** `npm install -D @codama/cli @codama/nodes-from-anchor @codama/renderers-js` (dev-deps).
- [ ] **Крок 21.** Створити `codama.json` руками (не запускати `npx codama init` — Windows ESM bug). Два renderer targets: `../BracketChain-Sdk/src/generated` + `../BracketChain-Indexer/src/generated`.
- [ ] **Крок 22.** Запустити `make codama-generate` (target вже є в Makefile, але зараз падає бо `codama.json` відсутній). Acceptance: `src/generated/` створені в обох sibling repos.
- [ ] **Крок 23.** Закоммітити згенерований код (per Solana convention) — це дає змогу CI gate детектити drift.
- [ ] CI gate (`.github/workflows/idl-check.yml`) — відкладений до cross-repo CI design; зараз — дисципліна local-regen.

### Section 3.4 — Tier-4 Anchor tests + CU baseline

- [ ] **Крок 15.** `tests/organizer-deposit.test.ts` — 3 тести:
  1. `organizerDeposit > 0` refund на pre-start cancel.
  2. Refund idempotency через `organizer_deposit_refunded` flag.
  3. Deposit excluded з prize-pool basis у `report_result` final-match.
- [ ] **Крок 16.** `anchor test` — всі 5 existing + 3 нові green.
- [ ] **Крок 17.** `tests/capacity-128p-deep.test.ts` — 128p full bracket → final. Розглянути `solana-bankrun-mocha` замість `solana-test-validator` (bankrun у секундах, не хвилинах).
- [ ] **Крок 18.** Вимірювати CU per ix через `getTransaction(sig, { maxSupportedTransactionVersion: 0 }).meta.computeUnitsConsumed`. Acceptance: кожен ix `< 1_400_000` CU.
- [ ] **Крок 19.** `CU_BUDGET.md` — baseline для всіх ix × preset. Phase 1 redeploy regression contract.

---

## Phase 1 — single redeploy bundle (~12-15 weeks solo)

### V1.1 — Game schema + SAS identity

- [ ] **Крок 43.** `src/state/game.rs` — `SupportedGame` enum (`Manual`/`Dota2`/`Cs2Faceit`/`Valorant`/`LoL`) + `SettlementMode` enum (`OrganizerOnly`/`PlayerReported`/`Oracle`). `Hybrid` **dropped**.
- [ ] **Крок 44.** Розширити `state/tournament.rs`: `game`, `settlement_mode`, `dispute_window_secs: u32`, `vrf_randomness_account: Pubkey`, `vrf_commit_slot: u64`, `seed_revealed: bool`.
- [ ] **Крок 45.** Розширити `state/participant.rs`: `identity_hash: [u8; 32]`, `identity_attestation: Pubkey`, `wins: u8`, `losses: u8`, `points_for: u32`, `points_against: u32` (cherry-picked сюди, ще не в формат-плані).
- [ ] **Крок 46.** Розширити `state/protocol_config.rs`: `sas_credential: Pubkey`, `sas_schema: Pubkey`.
- [ ] **Крок 47.** `instructions/set_sas_config.rs` — admin ix.
- [ ] **Крок 48.** Розширити `instructions/create_tournament.rs`: приймати `game` + `settlement_mode` + `dispute_window_secs`. Reject `Cs2Faceit`/`Valorant`/`LoL` з `GameNotYetSupported`.
- [ ] **Крок 49.** Розширити `instructions/join_tournament.rs`: якщо `tournament.game != Manual`, require `attestation: AccountInfo`. Validate owner == SAS program, credential + schema match `protocol_config`, nonce == player wallet, expiry. Extract `identity_bytes`, set `participant.identity_hash`. **Найвища складність V1.1**.
- [ ] **Крок 50.** `event_version: u8` як ПЕРШЕ поле в усіх 7 existing + V1+ `#[event]` structs. `EVENT_VERSION_V1 = 1`.

### V1 — Player-reported + VRF

- [ ] **Крок 53.** Розширити `state/match_node.rs`: proposal envelope (7 fields) + `bracket: u8` PDA seed (schema-prep для formats Phase B).
- [ ] **Крок 54.** `state/proposal_source.rs` — `ProposalSource` enum (`None`/`Player`/`Oracle`/`GameServer`).
- [ ] **Крок 55.** `instructions/request_seed.rs` — Switchboard randomness CPI, organizer-signed.
- [ ] **Крок 56.** `instructions/reveal_seed.rs` — permissionless, читає `RandomnessAccountData`.
- [ ] **Крок 57.** Модифікувати `start_tournament.rs`: gate на `seed_revealed` (skip для `OrganizerOnly`).
- [ ] **Крок 58.** `instructions/propose_result.rs` — signer ∈ {player_a, player_b}.
- [ ] **Крок 59.** `instructions/confirm_result.rs` — signer = counterparty.
- [ ] **Крок 60.** `instructions/dispute_result.rs` — signer = counterparty OR arbitrator (Oracle source).
- [ ] **Крок 61.** `instructions/claim_result.rs` — permissionless after `claim_deadline`.
- [ ] **Крок 62.** `instructions/resolve_dispute.rs` — organizer override.
- [ ] **Крок 63.** `instructions/force_claim_disputed.rs` — permissionless after 24h.
- [ ] **Крок 64.** `instructions/migrate_v1_tournament.rs` — devnet realloc (можна skip для fresh deploy).
- [ ] **Крок 65.** Comprehensive mocha tests для всіх 9 нових ix + VRF (mocked Switchboard).

### V1.2 — Switchboard Oracle settlement

- [ ] **Крок 70.** `state/match_commitment.rs` — `MatchCommitment` struct (lobby_id, player_a_game_id, player_b_game_id, committed_at, committed_slot). Додати `commitment: Option<MatchCommitment>` в `MatchNode`.
- [ ] **Крок 71.** `state/oracle_config.rs` — Switchboard program ID, queue, staleness threshold, arbitrator pubkey.
- [ ] **Крок 72.** `instructions/set_oracle_config.rs` — admin ix.
- [ ] **Крок 73.** `instructions/commit_match_lobby.rs` — organizer, validates both players мають identity_hash.
- [ ] **Крок 74.** `instructions/bind_match_feed.rs` — validates feed account ∈ Switchboard, OracleJob digest, store pubkey.
- [ ] **Крок 75.** `instructions/propose_result_oracle.rs` — permissionless, reads PullFeed, writes proposal envelope з `source = Oracle`.
- [ ] **Крок 76.** Розширити `dispute_result.rs`: Oracle source → signer ∈ {player_a, player_b, arbitrator}.
- [ ] **Крок 77.** Comprehensive mocha з MOCKED feeds.

### V1 — Program improvements + Partial cancel

- [ ] **Крок 80.** `PayoutPreset::Custom([u16; 8])` варіант + validation в `create_tournament` (sum == 10_000, no gaps, slots[0] > 0).
- [ ] **Крок 81.** `report_result.rs` final-match — runtime lookup замість hardcoded percentage tables, handle `PayoutPreset::Custom`.
- [ ] **Крок 82.** `instructions/close_tournament.rs` — permissionless, на `Completed`/`Cancelled`/`PartialCancelled`. Closes Tournament PDA + chunks MatchNode + Participant PDAs. Rent → original organizer. **B2C launch-critical** (без нього rent ~50% pool для малих турнірів).
- [ ] **Крок 84.** `TournamentStatus::PartialCancelled = 5`.
- [ ] **Крок 85.** `instructions/partial_cancel_tournament.rs` — organizer-signed, mid-Active.
- [ ] **Крок 86.** `instructions/partial_refund_chunk.rs` — permissionless, до ~10 participants/call, idempotent via `refund_paid` flag.

### Redeploy ceremony

- [ ] **Крок 88.** `anchor build` clean + CU regression vs Phase 0 baseline.
- [ ] **Крок 89.** `anchor deploy --provider.cluster devnet` → новий program ID. Запис в README + memory.
- [ ] **Крок 90.** `make codama-generate` → regenerated SDK + indexer clients.
- [ ] **Крок 93.** Bootstrap: `initialize_protocol` (новий ID), `set_sas_config`, `set_oracle_config`.

---

## Phase 4 — V1 Formats (optional, conditional on B2C signal)

- [ ] `TournamentFormat` enum (RoundRobin / DoubleElim / Swiss).
- [ ] Phase A: RR pairing (Circle method), `score_a`/`score_b` per-match.
- [ ] Phase B: DE — winners/losers/grand bracket, `bracket: u8` PDA seed (вже shipped в V1 player-reported).
- [ ] Phase C: Swiss — per-round pairing з `tournament.seed_hash` (вимагає V1 VRF).

## Phase 4 — V2-C GameServer attestation (B2B)

- [ ] Ed25519 attestations від game servers; organizer allowlist; writes V1's proposal envelope з `source = GameServer`.

---

## Sequencing constraints (від `roadmap.md`)

- **C1** V1.1 schema-prep ДО V1 player-reported.
- **C2** V1 player-reported ДО V1.2.
- **C3** V1 player-reported ДО V1 formats Phase C (Swiss).
- **C9** `bracket: u8` PDA seed ship в V1 player-reported (уникає 2-го redeploy для formats).
- **C10** `event_version: u8` як перше поле всіх `#[event]`.
