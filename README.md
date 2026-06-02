# BracketChain — Anchor Program

Solana on-chain tournament protocol with PDA-escrowed prize vaults and automatic preset-based payout distribution. Built with Anchor 0.32.1.

This repo contains only the smart contracts. The full system spans five repos — see [Related repositories](#related-repositories) below.

---

## Status

| Field | Value |
|---|---|
| Program ID — **MVP (live demo)** | `AuXJKpuZtkegs2ZSgopgckhN7Ev8bUz4zBc238LD2F1` — untouched; serves the Railway indexer + prod frontend |
| Program ID — **Phase 1 (dev/integration)** | `3YpkUKBh8288XN2dCKSwBnEdyc5UozSJ19A1ZCLpUZsZ` — current `declare_id!`; upgraded in-place as Stages A–E land. The full A–E feature set is **code-complete and built**; the Stage F promotion ceremony (re-deploy + 9 acceptance gates) is pending. |
| Cluster | devnet — [MVP](https://explorer.solana.com/address/AuXJKpuZtkegs2ZSgopgckhN7Ev8bUz4zBc238LD2F1?cluster=devnet) · [Phase 1 dev](https://explorer.solana.com/address/3YpkUKBh8288XN2dCKSwBnEdyc5UozSJ19A1ZCLpUZsZ?cluster=devnet) |
| Anchor | 0.32.1 |
| Solana | 2.x |
| Instructions | 24 (see [Instruction surface](#instruction-surface)) |
| Tests | 31 passing (ts-mocha + LiteSVM, see [Tests](#tests)) |
| IDL | `target/idl/bracket_chain.json` — single source of truth; sibling SDK + indexer client trees regenerate from it via `make codama-generate` |
| Upgrade authority | single key on devnet (`DeDQoza7…`) — Squads 2-of-3 multisig is a mainnet-prep gate |

---

## What it does

A single Anchor program. The **MVP** core (escrow + bracket + organizer-reported payout) was extended in **Phase 1** with three settlement modes, verifiable VRF seeding, Steam/SAS identity, custom payouts, and rent reclaim. Logical sub-systems:

- **Tournament Factory** — `initialize_protocol` creates the singleton `ProtocolConfig` PDA (fee bps, treasury, default mint, + Phase 1: SAS credential/schemas, Switchboard queue/staleness/min-samples). `create_tournament` takes `game` (`SupportedGame`), `settlement_mode` (`SettlementMode`), `dispute_window_secs`, `payout_preset` (now incl. `Custom([u16; 8])`), fees, deadline, optional organizer deposit.
- **Escrow Vault** — PDA token account at `[b"vault", tournament]` with `token::authority = tournament`. Accepts entry fees on `join_tournament` (non-`Manual` games require a valid SAS attestation account → `participant.identity_hash`). Refunds on `cancel_tournament`, `partial_refund_chunk`; rent reclaimed via `close_tournament`.
- **Verifiable seeding (VRF)** — `request_seed` (organizer) binds a committed Switchboard randomness account; `reveal_seed` (permissionless) reads it into `tournament.seed_hash`. `start_tournament` requires `seed_revealed` when a VRF account is bound (else falls back to the `slot_hashes` seed — keeps non-VRF flows locally testable).
- **Bracket Engine** — `start_tournament` idempotently inits all rounds across chunked txs (default 7 matches/chunk → 19 chunks for 128p; byes Completed at init). The `bracket: u8` PDA-seed byte is schema-prep for multi-format brackets (Phase 4). `report_result` is the OrganizerOnly path; the final-match branch distributes per preset and takes the 3.5% fee in-tx.
- **Settlement envelope (player / oracle)** — one proposal envelope on `MatchNode` (`proposal_source`, `proposer`, `proposed_winner`, `claim_deadline`, `disputed`, `dispute_reason`). `propose_result` (player) and `propose_result_oracle` (Switchboard relayer) both write it; `confirm_result` / `claim_result` / `dispute_result` / `resolve_dispute` / `force_claim_disputed` finalize it. The oracle is *just another proposer* — no parallel dispute system.

Status machine:

```
Registration ─► PendingBracketInit ─► Active ─► Completed
      │                                  │  │
      └──────────────► Cancelled ◄───────┘  └─► PartialCancelled
```

`cancel_tournament` is allowed from `Registration` / `PendingBracketInit`. From `Active`, the organizer can instead `partial_cancel_tournament` (→ `PartialCancelled`, then permissionless `partial_refund_chunk` refunds everyone — Policy A: full-refund-to-all). A reported match is otherwise irrevocable. Terminal states (`Completed` / `Cancelled` / `PartialCancelled`) can be `close_tournament`'d for rent reclaim.

---

## Instruction surface

24 instructions. **Core / admin:**

| # | Instruction | Effect |
|---|---|---|
| 1 | `initialize_protocol` | Creates singleton `ProtocolConfig` PDA. Idempotent. |
| 2 | `set_sas_config` | Admin: writes the SAS `credential` + `[Pubkey; 5]` game schemas into `ProtocolConfig`. |
| 3 | `set_oracle_config` | Admin: writes Switchboard `queue`, `max_stale_slots`, `min_oracle_samples`. |
| 4 | `migrate_protocol_config` | Admin: reallocs `ProtocolConfig` to the current `INIT_SPACE` (devnet upgrade-in-place). |
| 5 | `migrate_v1_tournament` | Admin: reallocs a Registration-phase `Tournament` to V1 layout (devnet upgrade-in-place; not needed for a fresh deploy). |

**Lifecycle:**

| # | Instruction | Effect |
|---|---|---|
| 6 | `create_tournament` | Creates `Tournament` PDA + vault TA. Takes `game`, `settlement_mode`, `dispute_window_secs`, `payout_preset` (incl. `Custom`, validated: sum 10_000, gapless, winner-funded), fees, deadline, optional organizer deposit. Emits `TournamentCreated`. |
| 7 | `join_tournament` | Transfers `entry_fee` → vault. For non-`Manual` games, requires a SAS attestation account (validated against `ProtocolConfig` credential/schema, nonce = player, expiry) → sets `participant.identity_hash`. Emits `ParticipantRegistered`. |
| 8 | `request_seed` | Organizer binds a committed Switchboard randomness account (`vrf_randomness_account` + `vrf_commit_slot`). |
| 9 | `reveal_seed` | Permissionless: reads the revealed randomness → `tournament.seed_hash`, sets `seed_revealed`. |
| 10 | `start_tournament` | Chunked (`chunk_index`, `total_chunks`). Gated on `seed_revealed` when VRF is bound. Idempotent; byes Completed at init. Flips → `Active`. Emits `TournamentStarted`. |
| 11 | `report_result` | **OrganizerOnly path.** Validates match `Active` + `winner ∈ {a,b}`. Non-final advances the winner; final distributes per preset over `vault − organizer_deposit`, refunds the deposit, takes 3.5% fee, → `Completed`. Emits `MatchReported` / `RefundIssued` / `TournamentCompleted`. |

**Player-reported settlement (envelope on `MatchNode`):**

| # | Instruction | Effect |
|---|---|---|
| 12 | `propose_result` | Player A/B proposes a winner; opens the dispute window (`claim_deadline = now + dispute_window_secs`). Emits `ResultProposed`. |
| 13 | `confirm_result` | Counterparty accepts → finalizes (credits `wins`/`losses`/points, advances / pays final). Emits `MatchReported`. |
| 14 | `dispute_result` | Counterparty (or arbitrator for Oracle source) disputes; re-arms `claim_deadline` +24h. Emits `ResultDisputed`. |
| 15 | `claim_result` | Permissionless: finalizes an undisputed proposal past its deadline. Emits `ResultClaimed` + `MatchReported`. |
| 16 | `resolve_dispute` | Organizer/arbitrator overrides the winner on a disputed match. Emits `DisputeResolved` + `MatchReported`. |
| 17 | `force_claim_disputed` | Permissionless backstop: finalizes a disputed match for the proposed winner after 24h of organizer silence. |
| 17a | `settle_final` | Arbitrator-signed finalize for non-`WinnerTakesAll` finals: winner is pinned to the trustless proposal; arbitrator adjudicates placements 3..N among semifinal losers. Emits `FinalSettled` + `MatchReported` + `TournamentCompleted`. |

**Oracle settlement (Switchboard On-Demand):**

| # | Instruction | Effect |
|---|---|---|
| 18 | `commit_match_lobby` | Organizer commits a match to a game `lobby_id` + player game IDs (`MatchCommitment`). Emits `MatchLobbyCommitted`. |
| 19 | `bind_match_feed` | Organizer binds a Switchboard PullFeed to a committed match (validates feed hash). Emits `MatchFeedBound`. |
| 20 | `propose_result_oracle` | Permissionless relayer reads the bound feed, verifies the winner hash vs the commitment, writes the envelope with `source = Oracle`. Emits `ResultProposed`. |

**Cancel / refund / cleanup:**

| # | Instruction | Effect |
|---|---|---|
| 21 | `cancel_tournament` | Pre-start two-tier cancel: organizer flips → `Cancelled`, then any signer drives refund chunks (idempotent via `refund_paid` / `organizer_deposit_refunded`). Emits `TournamentCancelled` + `RefundIssued`. |
| 22 | `partial_cancel_tournament` | Organizer-only, mid-`Active`: flips → `PartialCancelled`, freezes the bracket. Emits `TournamentPartiallyCancelled`. |
| 23 | `partial_refund_chunk` | Permissionless, chunked, idempotent: full entry-fee refund to every participant (Policy A); organizer recovers only their deposit. Emits `RefundIssued`. |
| 24 | `close_tournament` | Permissionless rent reclaim on a terminal status: closes child `MatchNode`/`Participant` PDAs in chunks, then optionally the vault + `Tournament` PDA; rent → organizer. Emits `TournamentClosed`. |

---

## Account model

All account types are PDAs. Seeds:

| Account | Seeds |
|---|---|
| `ProtocolConfig` | `[b"protocol_config"]` |
| `Tournament` | `[b"tournament", organizer.key, name.as_bytes()]` |
| `Participant` | `[b"participant", tournament.key, wallet.key]` |
| `MatchNode` | `[b"match", tournament.key, [bracket: u8], [round: u8], match_index.to_le_bytes() (u16)]` |
| Vault (SPL Token Account) | `[b"vault", tournament.key]` |

> **Phase 1 PDA-seed change (C9):** `MatchNode` gained a `bracket: u8` seed byte (`0` for single-elim) ahead of multi-format brackets. This re-addresses match PDAs vs the MVP layout — `migrate_v1_tournament` is scoped to Registration-phase tournaments only; in-progress tournaments surviving an upgrade-in-place must be cancelled & refunded (devnet only).

The vault is a PDA-owned `TokenAccount` (NOT an ATA). Its `mint` is the Tournament's `token_mint`, and its `authority` is the Tournament PDA itself, so the program can sign `token::transfer` CPIs to drain it on `report_result` payouts and `cancel_tournament` refunds.

`Tournament` fields (canonical):

```
organizer: Pubkey
name: String                    (≤ 32 bytes — see MAX_TOURNAMENT_NAME_LEN)
token_mint: Pubkey              (any SPL mint accepted; default_mint is advisory only)
vault: Pubkey                   (the PDA TA at [b"vault", tournament])
entry_fee: u64
organizer_deposit: u64
organizer_deposit_refunded: bool
max_participants: u16
bracket_size: u16               (next power of 2 ≥ max_participants; drives bye math)
participant_count: u16
matches_initialized: u16
matches_reported: u16
total_matches: u16              (= bracket_size − 1)
registration_deadline: i64
created_at: i64
started_at: i64
completed_at: i64
status: TournamentStatus
payout_preset: PayoutPreset     (WinnerTakesAll | Standard | Deep | Custom([u16; 8]))
seed_hash: [u8; 32]             (zero until start_tournament; from VRF or slot_hashes)
champion: Pubkey                (zero until final report_result)
bump: u8
vault_bump: u8
// ── Phase 1 (V1.1 + V1) ──
game: SupportedGame             (Manual | Dota2 | Cs2Faceit | Valorant | LoL)
settlement_mode: SettlementMode (OrganizerOnly | PlayerReported | Oracle)
dispute_window_secs: u32
vrf_randomness_account: Pubkey  (zero unless VRF seeding is bound)
vrf_commit_slot: u64
seed_revealed: bool
arbitrator: Pubkey              (dispute resolver; defaults to organizer)
```

**Phase 1 additions on the other accounts:**

- `Participant` += `identity_hash: [u8; 32]` (verbatim SAS `identity_bytes` = SHA-256(steam_id_64 LE); zero for `Manual`), `identity_attestation: Pubkey`, and foundation stats `wins`/`losses`/`points_for`/`points_against`.
- `MatchNode` += the proposal envelope (`proposal_source`, `proposer`, `proposed_winner`, `proposed_at`, `claim_deadline`, `disputed`, `dispute_reason`), `commitment: Option<MatchCommitment>` (oracle lobby), and `switchboard_feed: Pubkey`. Plus the `bracket: u8` seed byte (see above).
- `ProtocolConfig` += `sas_credential: Pubkey`, `sas_schemas: [Pubkey; 5]`, `switchboard_queue: Pubkey`, `max_stale_slots: u32`, `min_oracle_samples: u32`.

---

## Events

Emitted via Anchor `emit!()`. Indexer + SDK consume these via `BorshCoder` over the vendored IDL. **Every event carries `event_version: u8` (= `EVENT_VERSION_V1 = 1`) as its first field** — the indexer rejects any other version and increments an `unknownEventVersion` metric (gate G9).

| Event | Fired by |
|---|---|
| `TournamentCreated` | `create_tournament` |
| `ParticipantRegistered` | `join_tournament` |
| `TournamentStarted` | `start_tournament` (final chunk) |
| `MatchReported` | every finalize path (`report_result`, `confirm_result`, `claim_result`, `resolve_dispute`, `force_claim_disputed`) |
| `TournamentCompleted` | final-match finalize |
| `TournamentCancelled` | `cancel_tournament` (status flip) |
| `RefundIssued` | `cancel_tournament` / `partial_refund_chunk` (per refund; `kind` = entry-fee vs organizer-deposit) |
| `ResultProposed` | `propose_result`, `propose_result_oracle` |
| `ResultDisputed` | `dispute_result` |
| `ResultClaimed` | `claim_result` |
| `DisputeResolved` | `resolve_dispute` |
| `MatchLobbyCommitted` | `commit_match_lobby` |
| `MatchFeedBound` | `bind_match_feed` |
| `TournamentPartiallyCancelled` | `partial_cancel_tournament` |
| `TournamentClosed` | `close_tournament` |
| `FinalSettled` | `settle_final` |

`TournamentCompleted.placement_payouts` is a `Vec<PlacementPayout { recipient, amount, place }>` carried in the payload (avoids a transaction-log scan for payout rows).

---

## Constants & invariants

From `constants.rs`:

```rust
pub const PROTOCOL_FEE_BPS: u16 = 350;       // 3.5%
pub const MAX_PARTICIPANTS: u16 = 128;
pub const MIN_PARTICIPANTS: u16 = 2;
pub const MAX_TOURNAMENT_NAME_LEN: usize = 32;
pub const PROTOCOL_CONFIG_SEED: &[u8] = b"protocol_config";
pub const TOURNAMENT_SEED: &[u8]      = b"tournament";
pub const VAULT_SEED: &[u8]           = b"vault";
pub const PARTICIPANT_SEED: &[u8]     = b"participant";
pub const MATCH_SEED: &[u8]           = b"match";
```

Additional constants: `EVENT_VERSION_V1 = 1`, `FORCE_CLAIM_WINDOW_SECS = 86_400` (24h), `MAX_PAYOUT_SLOTS = 8`.

Enums:

```
TournamentStatus = Registration | PendingBracketInit | Active | Completed | Cancelled | PartialCancelled
MatchStatus      = Pending | Active | Completed
PayoutPreset     = WinnerTakesAll | Standard | Deep | Custom([u16; 8])
SupportedGame    = Manual=0 | Dota2=1 | Cs2Faceit=2 | Valorant=3 | LoL=4   (only Manual + Dota2 enabled)
SettlementMode   = OrganizerOnly=0 | PlayerReported=1 | Oracle=2
ProposalSource   = None=0 | Player=1 | Oracle=2 | GameServer=3
```

Payout split tables — `[u16; 8]`, sum to 10_000 bps before protocol fee:

| Preset | Slots (bps to placements 1..8) |
|---|---|
| `WinnerTakesAll` | `[10_000, 0, …]` |
| `Standard` | `[6_000, 2_500, 1_500, 0, …]` (60 / 25 / 15) |
| `Deep` | `[4_000, 2_500, 1_500, 1_000, 500, 300, 200, 0]` (40 / 25 / 15 / 10 / 5 / 3 / 2) |
| `Custom([u16; 8])` | Arbitrary — `validate_custom` requires sum == 10_000, gapless from index 0, `slots[0] > 0`. |

Each preset's funded (non-zero) slot count = `placement_count()`, which must be `≤ max_participants` — `create_tournament` rejects with `PresetExceedsParticipants` (or `InvalidCustomPayout` for a malformed `Custom` split) otherwise.

Net to placements: `prize_pool * (10_000 − PROTOCOL_FEE_BPS) / 10_000` = 96.5%.
Treasury: `prize_pool * PROTOCOL_FEE_BPS / 10_000` = 3.5%.

---

## Build

Anchor compilation must run on Linux/macOS or **WSL2 on Windows**. Native Windows builds are not supported by the Solana toolchain.

Required:
- Solana CLI 2.x (`sh -c "$(curl -sSfL https://release.solana.com/stable/install)"`)
- Anchor 0.32.1 (`avm install 0.32.1 && avm use 0.32.1`)
- Rust toolchain pinned by [`rust-toolchain.toml`](./rust-toolchain.toml)
- A funded keypair at `~/.config/solana/id.json` (devnet airdrop: `solana airdrop 5`)
- `pnpm` (used by SDK init script invoked from this Makefile) and `yarn` (Anchor scripts)

```bash
make build            # = anchor build (compiles + emits target/idl/bracket_chain.json)
make codama-generate  # = anchor build + npx codama run --all (regenerates SDK + indexer client trees)
```

After any IDL-affecting change run `make codama-generate` from this repo with `../BracketChain-Sdk` and `../BracketChain-Indexer` checked out as sibling peers. The generated trees land at `../BracketChain-Sdk/src/generated/` and `../BracketChain-Indexer/src/generated/` and must be committed alongside the program diff so the consumers' typed accounts/instructions stay in lockstep with on-chain layout. Event decoding still goes through `BorshCoder` over `target/idl/bracket_chain.json` (Codama's v2 renderers don't emit event decoders yet), so a fresh `target/idl/` is also required by the indexer parser.

---

## Tests

```bash
anchor test           # boots local validator, runs ts-mocha across all tests/*.ts, tears down
```

**31 passing** across two tiers — validator-backed integration tests and fast in-process [LiteSVM](https://github.com/LiteSVM/litesvm) instruction-surface tests:

| File | Tier | Covers |
|---|---|---|
| `tests/bracket-chain.ts` | validator | WTA + Standard happy paths, cancel+refund, bye bracket, 128p chunked start; emits the WTA/Standard/Cancel CU lines |
| `tests/organizer-deposit.test.ts` | validator | `organizer_deposit` refund on pre-start cancel, refund idempotency, Variant-A refund-on-final |
| `tests/capacity-128p-deep.test.ts` | validator | 128p Deep full bracket → final; samples the CU baseline (`CU_BUDGET.md`) |
| `tests/player-reported.test.ts` | validator | propose+confirm advancement + stats + final payout; dispute → organizer resolve; permissionless `claim_result`; OrganizerOnly rejection; force-claim/early-claim guards |
| `tests/oracle-litesvm.test.ts` | LiteSVM | `propose_result_oracle` (feed → winner), wrong-feed / already-proposed rejects, `bind_match_feed` hash mismatch, Oracle-source dispute broadening |
| `tests/partial-cancel-litesvm.test.ts` | LiteSVM | `partial_cancel_tournament` Active→PartialCancelled + auth/state guards; `partial_refund_chunk` state guard + idempotency |
| `tests/close-tournament-litesvm.test.ts` | LiteSVM | terminal-gate, child-close + rent-to-organizer, idempotency, wrong-child / wrong-organizer rejects |

`tests/utils.ts` exposes `sendStartChunks` (wraps each chunk with `ComputeBudgetProgram.setComputeUnitLimit(1_400_000)` — the SDK's `startTournament` pattern) and `measureCu`.

`CU_BUDGET.md` is the regression contract — Phase 1 redeploy ceremony re-runs the suite and diffs computeUnitsConsumed against the file. Drift triggers a review before re-publishing the SDK.

## Codama codegen

The SDK + Indexer consume typed accounts/instructions generated from `target/idl/bracket_chain.json` by [`@codama/cli`](https://github.com/codama-idl/codama) using `@codama/renderers-js@2.x` (which emits the `@solana/kit`-style package-wrapped layout). Configuration lives at `codama.json`.

```bash
make codama-generate   # rebuilds IDL, regenerates ../BracketChain-Sdk/src/generated and ../BracketChain-Indexer/src/generated
```

Sibling repos expected as peers (`../BracketChain-Sdk`, `../BracketChain-Indexer`). The generated trees are committed in each consumer repo so a CI gate can detect drift — drift gate itself is deferred to cross-repo CI design.

---

## Deploy

Devnet only in MVP. Mainnet deploy is gated on migration to a Squads 2-of-3 multisig upgrade authority — see the main repo's MVP-vs-V1 deltas.

```bash
# 1. Compile + emit IDL
anchor build

# 2. Deploy to devnet (uses Anchor.toml [provider] keypair + cluster)
anchor deploy --provider.cluster devnet

# 3. Initialize ProtocolConfig singleton — idempotent (reads first, skips if already initialized)
cd ../BracketChain-Sdk
pnpm tsx scripts/init-protocol.ts --rpc=https://api.devnet.solana.com
# or with a custom RPC:
# pnpm tsx scripts/init-protocol.ts --rpc=https://devnet.helius-rpc.com/?api-key=YOUR_KEY
```

After a fresh deploy, regenerate Codama trees + republish the SDK if the IDL changed:

```bash
make codama-generate
cd ../BracketChain-Sdk && pnpm build && pnpm publish --access public
```

---

## Repository layout

```
.
├── Anchor.toml              # cluster config, program IDs, scripts
├── Cargo.toml               # workspace
├── Makefile                 # build / idl / codama-generate recipes (sync-idl is a deprecation alias)
├── programs/
│   └── bracket-chain/
│       ├── Cargo.toml
│       └── src/
│           ├── lib.rs           # entrypoint — declare_id + #[program] handlers
│           ├── constants.rs
│           ├── errors.rs
│           ├── events.rs
│           ├── instructions/    # one file per instruction
│           └── state/           # one file per account type
├── tests/
│   ├── bracket-chain.ts             # core happy paths + CU lines (validator)
│   ├── organizer-deposit.test.ts    # deposit-flow (Variant A) (validator)
│   ├── capacity-128p-deep.test.ts   # 128p Deep bracket + CU sampling (validator)
│   ├── player-reported.test.ts      # propose/confirm/dispute/claim/resolve (validator)
│   ├── oracle-litesvm.test.ts       # oracle settlement (LiteSVM)
│   ├── partial-cancel-litesvm.test.ts # partial cancel + refund (LiteSVM)
│   ├── close-tournament-litesvm.test.ts # rent reclaim (LiteSVM)
│   └── utils.ts                     # test helpers (compute-budget wrap, measureCu, ATA setup)
├── CU_BUDGET.md             # compute-unit baseline — regression contract for redeploy
├── codama.json              # Codama codegen config for sibling SDK + indexer
├── target/                  # gitignored — anchor build output, IDL
├── migrations/              # Anchor migrations dir (unused — see Makefile comments)
├── app/                     # Anchor scaffolding placeholder (unused)
└── rust-toolchain.toml      # pinned Rust toolchain
```

Init / deploy scripts deliberately live in [`bracket-chain-sdk/scripts/init-protocol.ts`](../bracket-chain-sdk/scripts/init-protocol.ts) (single source of truth for IDL + PDA helpers), not in `migrations/`. The Anchor `migrations/deploy.ts` hook is treated as deprecated by every serious Solana protocol — see the rationale at the top of [`Makefile`](./Makefile).

---

## Related repositories

| Repo | Purpose |
|---|---|
| [`bracketchain-main`](../bracketchain-main) | Top-level README, hackathon plan, MVP-vs-V1 deltas, demo script |
| [`bracket-chain-sdk`](../bracket-chain-sdk) | TypeScript SDK — published as [`@bracketchain/sdk`](https://www.npmjs.com/package/@bracketchain/sdk). Wraps this program for transaction construction, account fetching, and WebSocket subscriptions. |
| [`bracket-chain-indexer`](../bracket-chain-indexer) | NestJS read API + Helius webhook ingestor. Backs the `/explore` listing and stale-while-revalidate reads on `/t/[id]`. |
| [`BracketChain-Frontend`](../BracketChain-Frontend) | Next.js 16 web app — wallet adapter, create / join / view / dashboard. |

---

## License

MIT. See [`LICENSE`](./LICENSE).
