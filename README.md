# BracketChain — Anchor Program

Solana on-chain tournament protocol with PDA-escrowed prize vaults and automatic preset-based payout distribution. Built with Anchor 0.32.1.

This repo contains only the smart contracts. The full system spans five repos — see [Related repositories](#related-repositories) below.

---

## Status

| Field | Value |
|---|---|
| Program ID — **MVP (live demo)** | `AuXJKpuZtkegs2ZSgopgckhN7Ev8bUz4zBc238LD2F1` — untouched; serves the Railway indexer + prod frontend |
| Program ID — **Phase 1 (dev/integration)** | `3YpkUKBh8288XN2dCKSwBnEdyc5UozSJ19A1ZCLpUZsZ` — current `declare_id!`; upgraded in-place as Stages B–E land. Promoted to production at the Stage F ceremony. |
| Cluster | devnet — [MVP](https://explorer.solana.com/address/AuXJKpuZtkegs2ZSgopgckhN7Ev8bUz4zBc238LD2F1?cluster=devnet) · [Phase 1 dev](https://explorer.solana.com/address/3YpkUKBh8288XN2dCKSwBnEdyc5UozSJ19A1ZCLpUZsZ?cluster=devnet) |
| Anchor | 0.32.1 |
| Solana | 2.x |
| Tests | 10/10 passing (ts-mocha, see [Tests](#tests)) |
| IDL | `target/idl/bracket_chain.json` — single source of truth; sibling SDK + indexer client trees regenerate from it via `make codama-generate` |
| Upgrade authority | single key on devnet — Squads 2-of-3 multisig is a mainnet-prep gate, not in MVP |

---

## What it does

A single Anchor program implementing three logical sub-systems behind six instructions:

- **Tournament Factory** — `initialize_protocol` creates the singleton `ProtocolConfig` PDA (fee basis points, treasury, advisory default mint). `create_tournament` creates per-tournament PDAs with config (preset, fees, deadline, optional organizer deposit) and a PDA-owned vault token account.
- **Escrow Vault** — a PDA token account at `[b"vault", tournament]` with `token::authority = tournament` so the Tournament PDA signs CPIs out. Accepts entry fees on `join_tournament`. Refunds fees + organizer deposit on `cancel_tournament`.
- **Bracket Engine** — `start_tournament` captures `seed_hash` from the `slot_hashes` sysvar and idempotently initializes all rounds across chunked transactions (default 7 matches per chunk → 19 chunks for 128 players; bye matches mark Completed at init). `report_result` enforces `MatchStatus = Active` and winner ∈ {playerA, playerB}. The final-match branch validates 1st and 2nd on-chain (3rd–Nth placements are organizer-trusted in MVP), distributes prize per preset, and takes a 3.5% protocol fee in the same transaction.

Status machine:

```
Registration ─► PendingBracketInit ─► Active ─► Completed
      │                                  │
      └────────────────► Cancelled ◄─────┘
```

Cancellation is allowed from `Registration` or `PendingBracketInit`. From `Active` it is rejected on-chain (any reported match is irrevocable).

---

## Instruction surface

| # | Instruction | Args | Effect |
|---|---|---|---|
| 1 | `initialize_protocol` | `fee_bps: u16, treasury: Pubkey, default_mint: Pubkey` | Creates singleton `ProtocolConfig` PDA. Idempotent (skipped by SDK init script if already initialized). |
| 2 | `create_tournament` | `name: String, entry_fee: u64, max_participants: u16, payout_preset: PayoutPreset, registration_deadline: i64, organizer_deposit: u64` | Creates `Tournament` PDA + PDA-owned vault TA. CPI from organizer's ATA → vault when `organizer_deposit > 0` (organizer ATA passed as optional account). Emits `TournamentCreated { ..., name, organizer_deposit }`. |
| 3 | `join_tournament` | — | Transfers `entry_fee` from joiner's ATA → vault via SPL token CPI. Creates `Participant` PDA. Emits `ParticipantRegistered`. |
| 4 | `start_tournament` | `chunk_index: u8, total_chunks: u8` | First call captures `seed_hash` from `slot_hashes[1..N]`, derives seeded bracket order, and inits matches in chunks. Idempotent — re-running a chunk that already ran is a no-op. Bye matches Completed at init (winner = the one player). Flips status to `Active` after the final chunk. Emits `TournamentStarted`. |
| 5 | `report_result` | `winner: Pubkey, score_a: u16, score_b: u16` | Validates match `Active` and `winner ∈ {a, b}`. Non-final advances winner to next round's match slot. Final match: refunds `organizer_deposit` back to organizer (Variant A — deposit is excluded from the prize-pool basis), then distributes prize across placements per `PayoutPreset` (3rd–Nth recipients passed by organizer, validated against participant set) over `vault.amount − organizer_deposit`, takes 3.5% to treasury, flips status to `Completed`. Emits `MatchReported` per match + `RefundIssued` on the deposit refund + `TournamentCompleted { placement_payouts, treasury_recipient }` on final. The optional `organizer_token_account` account is required only when `organizer_deposit > 0`. |
| 6 | `cancel_tournament` | — | Two-tier authorization: organizer flips status to `Cancelled` (first call); any signer can drive subsequent refund chunks. Refunds entry fees back to participant ATAs + `organizer_deposit` back to organizer ATA (idempotent via `organizer_deposit_refunded` flag). Emits `TournamentCancelled` + `RefundIssued` per refund. Rejects from `Active` (matches already reported). |

---

## Account model

All account types are PDAs. Seeds:

| Account | Seeds |
|---|---|
| `ProtocolConfig` | `[b"protocol_config"]` |
| `Tournament` | `[b"tournament", organizer.key, name.as_bytes()]` |
| `Participant` | `[b"participant", tournament.key, wallet.key]` |
| `MatchNode` | `[b"match", tournament.key, [round: u8], match_index.to_le_bytes() (u16)]` |
| Vault (SPL Token Account) | `[b"vault", tournament.key]` |

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
payout_preset: PayoutPreset
seed_hash: [u8; 32]             (zero until start_tournament; set from slot_hashes)
champion: Pubkey                (zero until final report_result)
bump: u8
vault_bump: u8
```

---

## Events

Emitted via Anchor `emit!()`. Indexer + SDK consume these via `BorshCoder` over the vendored IDL.

| Event | Fired by |
|---|---|
| `TournamentCreated { tournament, organizer, name, token_mint, entry_fee, max_participants, payout_preset, registration_deadline, organizer_deposit }` | `create_tournament` |
| `ParticipantRegistered { tournament, wallet, seed_index }` | `join_tournament` |
| `TournamentStarted { tournament, seed_hash }` | `start_tournament` (final chunk only) |
| `MatchReported { tournament, round, match_index, winner, score_a, score_b }` | `report_result` |
| `TournamentCompleted { tournament, champion, placement_payouts, treasury_recipient }` | `report_result` (final match) |
| `TournamentCancelled { tournament }` | `cancel_tournament` (first call, status flip) |
| `RefundIssued { tournament, recipient, amount, kind }` | `cancel_tournament` (per refund chunk; `kind` distinguishes entry-fee refund from organizer-deposit refund) |

`TournamentCompleted.placement_payouts` is a `Vec<PlacementPayout { recipient, amount, place }>` carried in the event payload (Phase 5.2 path D — fixes the earlier indexer payout-row gap by avoiding a transaction-log scan).

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

Enums:

```
TournamentStatus = Registration | PendingBracketInit | Active | Completed | Cancelled
MatchStatus      = Pending | Active | Completed
PayoutPreset     = WinnerTakesAll | Standard | Deep
```

Payout split tables (sum to 10_000 bps before protocol fee):

| Preset | Slots (bps to placements 1..N) |
|---|---|
| `WinnerTakesAll` | `[10_000]` |
| `Standard` | `[6_000, 2_500, 1_500]` (60 / 25 / 15) |
| `Deep` | `[4_000, 2_500, 1_500, 1_000, 500, 300, 200]` (40 / 25 / 15 / 10 / 5 / 3 / 2) |

Each preset's non-zero slot count must be `≤ max_participants` — `create_tournament` rejects with `PresetExceedsParticipants` otherwise.

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

Ten tests across three files:

| File | # | Scenario | Verifies |
|---|---|---|---|
| `tests/bracket-chain.ts` | 1 | WTA 8-player happy path | Single payout = 96.5% of prize pool to champion; 3.5% to treasury |
| | 2 | Standard 8-player happy path | 60/25/15 split + fee math |
| | 3 | Cancel + refund (4 players) | Vault drains to zero; all 4 entry fees returned to original ATAs |
| | 4 | Bye 7-player tournament | Non-power-of-2 bracket; bye matches Completed at init; advancement works |
| | 5 | 128-player chunked start | `start_tournament` succeeds across 19 chunks; per-chunk compute budget under limit; status flips to `Active` only after final chunk |
| `tests/organizer-deposit.test.ts` | 6 | Pre-start deposit refund on cancel | `organizer_deposit > 0` flow: cancel from `Registration` returns deposit + every entry fee |
| | 7 | Refund idempotency | Re-running cancel chunks is a no-op via `organizer_deposit_refunded` flag |
| | 8 | Variant A deposit refund on final | `report_result` final match refunds deposit out of band, distributes prize on `vault.amount − organizer_deposit` |
| `tests/capacity-128p-deep.test.ts` | 9 | 128p Deep full-bracket → final | Bracket inits + every match reports + 7-slot payout distributes correctly |
| | 10 | CU baseline sampling | `meta.computeUnitsConsumed` recorded for representative ix; baseline lives in `CU_BUDGET.md` |

`tests/utils.ts` exposes `sendStartChunks` which wraps each chunk tx with `ComputeBudgetProgram.setComputeUnitLimit(1_400_000)`. Treat this as the SDK's pattern for the `startTournament` flow.

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
│   ├── bracket-chain.ts          # 5 demo-path tests
│   ├── organizer-deposit.test.ts # 3 deposit-flow tests (Variant A)
│   ├── capacity-128p-deep.test.ts# 2 tests — 128p Deep bracket + CU sampling
│   └── utils.ts                  # test helpers (compute-budget wrap, ATA setup, etc.)
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
