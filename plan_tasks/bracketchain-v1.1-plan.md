# BracketChain V1.1 — Phase 1: Game-Aware Schema + SAS Identity Layer

## Context

BracketChain's MVP shipped on devnet at `AuXJKpuZtkegs2ZSgopgckhN7Ev8bUz4zBc238LD2F1` with organizer-attested match results (`report_result` is a `Signer == tournament.organizer` instruction). The next strategic capability is **trust-minimized settlement** for PC esports tournaments — players play a real Dota 2 / CS2 / Valorant match, an oracle reports the winner, the vault pays out automatically.

That full capability spans multiple workstreams (oracle wiring, dispute windows, multi-game adapters). This plan covers **only the foundation**: the schema unification and the wallet↔game-identity attestation layer that everything else depends on. After this lands, V1 can add the Switchboard oracle path on Dota 2 in a focused follow-up without further schema churn.

**Scope decisions (locked):**
- **Phase 1 only**: schema + identity. No oracle wiring in this plan.
- **SAS attestations** for wallet↔game-identity binding (composable, no on-chain bloat in BracketChain's own program).
- **Oracle-default for game-tagged tournaments**: when `game != Manual`, both the on-chain field default and the create-wizard UI default to `settlement_mode = Oracle`. Behavior remains manual-only in Phase 1 (no oracle ix yet) — `report_result` honors the existing organizer-signer path regardless of `settlement_mode`. Phase 2 introduces the runtime gate.
- **Drop the `Hybrid` variant from `SettlementMode`.** Originally reserved across V1.1 / V1 player-reported / V1.2 / V2 with no fixed semantics ("multi-source consensus"? "primary + fallback"? — never specified). Each plan would have to handle a dead arm in every `match settlement_mode` and a dead string in every TypeScript union. Removing it now is wire-safe (zero on-chain Tournaments use it) and frees discriminator `3` for `GameServer` in V2 (contiguous layout, no gap). If a real hybrid use case emerges later, it ships as a well-specified variant in a dedicated plan with concrete timeout/authority/override rules — not as a placeholder reservation.
- **Add `event_version: u8` as the first field of every `#[event]` struct in this redeploy.** Borsh decoding is positional — if the vendored IDL on the indexer side drifts from the program by even a single field, the parser silently decodes garbage values that don't crash. Adding a leading version byte to every event lets the indexer **hard-reject mismatched versions** (`if event_version != EXPECTED_VERSION { return SkipWithLog; }`) instead of silently writing garbage to the database. Cost: 1 byte per event payload (negligible) + 30 lines of indexer parser code that reads version first. Why now, not later: retrofitting after production deployment requires a coordinated frontend + indexer + SDK rollout with parser-version-pinning logic; adding the byte at this Phase 1 redeploy is free. Applies to all 7 MVP events (`TournamentCreated`, `ParticipantRegistered`, `TournamentStarted`, `MatchReported`, `TournamentCompleted`, `TournamentCancelled`, `RefundIssued`) plus every event added by V1 player-reported / V1.2 / V1 partial-cancel / V1 program-improvements in the same redeploy. Initial value: `EVENT_VERSION_V1 = 1`. Bumped per event only when its struct shape changes incompatibly (additive trailing fields don't bump; reordering / type-changing fields do).

**Out of scope for this plan** (explicit list at the bottom): Switchboard feeds, `report_result_oracle`, dispute windows, CS2/Valorant adapters, BR placement payouts, Squads arbitration.

**Canonical next plan: V1 player-reported (`bracketchain-v1-player-reported-plan.md`).** That plan owns the proposal/dispute envelope, the VRF primitive (`request_seed` / `reveal_seed`), and the rename of `Manual` → `OrganizerOnly`. Together V1.1 + V1 player-reported form the **minimum viable real-money tournament stack** for PC esports (Dota 2 / CS2): players authenticate via Steam OpenID (this plan), report results trustlessly with dispute window (V1 player-reported), and bracket seeding is unmanipulable via VRF (V1 player-reported). Bundle both into a single program redeploy where possible — devnet doesn't owe state continuity, and a paired redeploy halves the migration ceremony. V1.2 (Oracle) and V1 formats Phase C (Swiss) chain after V1 player-reported because they depend on its VRF surface; see those plans' sequencing notes.

---

## Architecture

```
                      ┌──────────────────────────────────────────┐
                      │  Solana Attestation Service (SAS)        │
                      │  Program: 22zoJMtdu4tQc2PzL74ZUT7Frwg…   │
                      │                                          │
                      │  Credential PDA: bracketchain-issuer     │
                      │    └─ authorized_signer = indexer key    │
                      │  Schema PDA: bracketchain.game_identity  │
                      │    └─ fields: { game: u8, steam_id_64,   │
                      │                  identity_bytes[32] }    │
                      │  Attestation PDA per { wallet, schema }  │
                      └──────────────────────────────────────────┘
                                       ▲                    ▲
                                       │ create_attestation │ verify (read)
                                       │ (CPI from indexer) │
                                       │                    │
                      ┌────────────────┴───┐   ┌────────────┴────────────────┐
                      │  Indexer (Nest)    │   │  BracketChain program        │
                      │  /identity/steam/* │   │  join_tournament (extended)  │
                      │  + sas-issuer svc  │   │  - if tournament.game != 0   │
                      │  - Steam OpenID    │   │    require attestation acct  │
                      │  - SAS CPI         │   │    validate owner=SAS,       │
                      │  - holds 1 keypair │   │      credential, schema,     │
                      └────────────────────┘   │      nonce=player, expiry    │
                                               │    extract identity_bytes    │
                                               │    store identity_hash       │
                                               └──────────────────────────────┘
```

The union design lives in the on-chain enums (`SupportedGame`, `SettlementMode`) and in the canonical `identity_hash: [u8; 32]` on Participant. Per-game adapters land in Phase 2 as Switchboard feed-job templates; Phase 1 is game-agnostic except for the schema-PDA lookup in `join_tournament`.

---

## Program changes (`bracket-chain-programs/`)

### New file: `programs/bracket-chain/src/state/game.rs`

```rust
use anchor_lang::prelude::*;

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, Debug, InitSpace)]
pub enum SupportedGame {
    Manual,        // 0 — current MVP behavior, no identity required
    Dota2,         // 1
    Cs2Faceit,     // 2 — placeholder; no joins allowed until Phase 2
    Valorant,      // 3 — placeholder
    LoL,           // 4 — placeholder
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, Debug, InitSpace)]
pub enum SettlementMode {
    Manual,        // 0 — organizer signer → report_result
    Oracle,        // 1 — reserved; honored as Manual in Phase 1
    // No `Hybrid` variant: see Scope decisions (locked) for rationale.
    // V1 player-reported plan inserts PlayerReported between these,
    // shifting Oracle from discriminator 1 → 2 (accepted as part of fresh-redeploy migration).
}
```

> **Cross-plan note**: V1 (`bracketchain-v1-player-reported-plan.md`) renames `Manual` → `OrganizerOnly` and adds a `PlayerReported` variant; discriminator `0` is unchanged, so the rename is IDL-only and ships before V1.2.

Re-export in `state/mod.rs`. The Cs2/Valorant/LoL variants exist so the IDL is stable across Phase 2 — `create_tournament` rejects them with `BracketChainError::GameNotYetSupported` until their adapter ships.

### Modify: `state/tournament.rs`

Add after `payout_preset`:
```rust
pub game: SupportedGame,
pub settlement_mode: SettlementMode,
```
Account-size cost: `+1 +1` = 2 bytes per Tournament. `InitSpace` macro handles this automatically since the enums derive `InitSpace`.

### Modify: `state/participant.rs`

Currently 7 fields. Add:
```rust
pub identity_hash: [u8; 32],
pub identity_attestation: Pubkey,

// Foundation stats — cherry-picked from V1 formats plan Phase A so partial-cancel,
// future formats plan, and webapp Phase D badge logic share one shipping point.
pub wins: u8,
pub losses: u8,
pub points_for: u32,
pub points_against: u32,
```
For `game == Manual` tournaments, the identity fields are zero-bytes (sentinel). For game-tagged tournaments, populated at `join_tournament` time. Stats fields zero-initialize on registration and are incremented by `report_result` (and V1's `confirm_result` / `claim_result` / `resolve_dispute` / `force_claim_disputed` — every path that finalizes a match). Account-size cost: +64 + 10 = **74 bytes per participant**.

**Why these stats live here, not in V1 formats plan Phase A:** three downstream plans consume them:
- **V1 partial-cancel** uses `losses == 0` as the O(1) survivor check. Without this field at Phase 1, partial-cancel falls back to iterating all `MatchNode` PDAs per participant — ~10× CU cost and chunked execution.
- **V1 formats plan Phase A** uses them for RR's wins → head-to-head → differential tiebreaker. The formats plan ships only the RR/DE/Swiss-specific extensions (`TournamentFormat` enum, `bracket: u8` PDA seed, `score_a` / `score_b` per match, `start_round` ix for Swiss); the stats fields themselves are pre-shipped here.
- **V1 webapp plan Phase D** uses them for badge-eligibility cron (`first_win`, `win_streak_5`, `won_5_tournaments`, etc.). Webapp Phase D's eligibility cron reads `player_stats` view, which aggregates these fields.

Shipping the stats in V1.1 means one shared foundation rather than three separate Participant-struct extensions, each of which would require its own program redeploy if shipped independently. The cost is 10 extra bytes per Participant in Phase 1; the payoff is no partial-cancel fallback path, no formats-Phase-A dependency cascade, and no Phase D scrambling to backfill stats.

### Modify: `state/protocol_config.rs`

Add:
```rust
pub sas_credential: Pubkey,                          // BracketChain's SAS Credential PDA
pub sas_schemas: [Pubkey; 5],                        // one schema PDA per SupportedGame variant
```
Populated via a new `set_sas_config` ix called by the protocol authority post-deploy.

### Modify: `instructions/create_tournament.rs`

Extend handler signature with `game: SupportedGame, settlement_mode: SettlementMode`. Validation:
- Reject `Cs2Faceit | Valorant | LoL` with `GameNotYetSupported` (Phase 1 ships Dota 2 only).
- If `game == Manual`, force `settlement_mode = Manual`.
- If `game == Dota2`, allow `Oracle` (stored as metadata; Phase 1 runtime still gates on organizer signer).

Update `lib.rs` `create_tournament` entrypoint params. Update `TournamentCreated` event to include `game` and `settlement_mode`.

### Modify: `events.rs`

**Two changes apply to every event struct in this redeploy** (not just the ones V1.1 extends):

1. **Add `event_version: u8` as the first field** of every `#[event]` struct (per Scope decisions locked above). Initial value `EVENT_VERSION_V1 = 1`. Define constant in `constants.rs`.
2. **Extend the events this plan touches** with their V1.1-specific fields.

Concrete event shapes after V1.1:

```rust
#[event]
pub struct TournamentCreated {
    pub event_version: u8,                // NEW — must be first; rejected by indexer if != EVENT_VERSION_V1
    pub tournament: Pubkey,
    pub organizer: Pubkey,
    pub name: String,                     // ≤32 bytes (MAX_TOURNAMENT_NAME_LEN)
    pub token_mint: Pubkey,
    pub entry_fee: u64,
    pub max_participants: u16,
    pub payout_preset: PayoutPreset,
    pub registration_deadline: i64,
    pub organizer_deposit: u64,
    pub game: SupportedGame,              // NEW (V1.1)
    pub settlement_mode: SettlementMode,  // NEW (V1.1)
}

#[event]
pub struct ParticipantRegistered {
    pub event_version: u8,                // NEW
    pub tournament: Pubkey,
    pub wallet: Pubkey,
    pub seed_index: u16,
    pub identity_hash: [u8; 32],          // NEW (V1.1) — zero-bytes for Manual-game tournaments
}
```

The other 5 MVP events (`TournamentStarted`, `MatchReported`, `TournamentCompleted`, `TournamentCancelled`, `RefundIssued`) keep their MVP shape but **also gain `event_version: u8` as their first field** — they're not extended by V1.1 logically, but the version byte is added uniformly across all events in the redeploy so the indexer parser can apply one rule everywhere.

**Indexer parser pattern** (relevant for `bracket-chain-indexer/src/webhooks/helius-parser.service.ts`):

```ts
const EVENT_VERSION_V1 = 1;

function decodeEvent(eventName: string, payload: Buffer): DecodedEvent | null {
  const version = payload.readUInt8(0);
  if (version !== EVENT_VERSION_V1) {
    logger.warn(`Skipping ${eventName} with unknown event_version=${version}`);
    metrics.unknownEventVersion.inc({ event: eventName, version });
    return null;  // do NOT silently decode the rest
  }
  return borshCoder.decode(eventName, payload);
}
```

This is the load-bearing safety check that distinguishes "wire-incompatible drift" from "data we know how to decode."

### Modify: `instructions/join_tournament.rs`

Reuses the existing `JoinTournament<'info>` accounts struct. Add one **optional** account at the end:
```rust
/// CHECK: validated against tournament.game and protocol_config.sas_schemas[game] below.
/// Required when tournament.game != Manual; pass default Pubkey otherwise.
pub game_identity_attestation: Option<UncheckedAccount<'info>>,
pub protocol_config: Account<'info, ProtocolConfig>,
```

Handler logic insertion (after the existing `TournamentStatus::Registration` check, before the token transfer):

```rust
if tournament.game != SupportedGame::Manual {
    let attestation_ai = ctx.accounts.game_identity_attestation
        .as_ref()
        .ok_or(BracketChainError::AttestationRequired)?;

    // 1. Owner check
    require_keys_eq!(*attestation_ai.owner, SAS_PROGRAM_ID, BracketChainError::InvalidAttestationOwner);

    // 2. Deserialize SAS Attestation via shared crate or manual layout
    let data = attestation_ai.try_borrow_data()?;
    let attestation = Attestation::try_from_bytes(&data)?;

    // 3. Credential + schema match BracketChain's
    require_keys_eq!(attestation.credential, ctx.accounts.protocol_config.sas_credential,
        BracketChainError::WrongAttestationCredential);
    let expected_schema = ctx.accounts.protocol_config.sas_schemas[tournament.game as usize];
    require_keys_eq!(attestation.schema, expected_schema, BracketChainError::WrongAttestationSchema);

    // 4. Nonce binds attestation to this wallet
    require_keys_eq!(Pubkey::try_from(&attestation.nonce[..])?, ctx.accounts.player.key(),
        BracketChainError::AttestationWalletMismatch);

    // 5. Not expired
    require!(now < attestation.expiry, BracketChainError::AttestationExpired);

    // 6. Hash the identity bytes (positions 0..32 of attestation.data per schema)
    let identity_hash = keccak::hash(&attestation.data[0..32]).0;
    participant.identity_hash = identity_hash;
    participant.identity_attestation = attestation_ai.key();
}
```

Update `ParticipantRegistered` event to include `identity_hash` (zero-bytes for Manual tournaments).

### Modify: `errors.rs`

Add:
```rust
GameNotYetSupported,
AttestationRequired,
InvalidAttestationOwner,
WrongAttestationCredential,
WrongAttestationSchema,
AttestationWalletMismatch,
AttestationExpired,
SasConfigNotInitialized,
```

### New ix: `instructions/set_sas_config.rs`

Authority-gated (protocol_config.authority signer). Writes `sas_credential` and `sas_schemas[]` to `ProtocolConfig`. One-time bootstrap; idempotent (allows updates). Mirrors the existing protocol-admin pattern.

### Constants

In `constants.rs` add:
```rust
pub const SAS_PROGRAM_ID: Pubkey = pubkey!("22zoJMtdu4tQc2PzL74ZUT7FrwgB1Udec8DdW4yw4BdG");
```

### Dependencies

`Cargo.toml` in `programs/bracket-chain/`:
- Add `solana-attestation-service = { version = "<latest>", features = ["no-entrypoint"] }` for the `Attestation` deserialization helper. **User must confirm/install** — do not edit `Cargo.toml` proactively beyond listing the dep.
- Alternative if SAS doesn't ship a no-entrypoint crate yet: implement manual byte-layout parsing in BracketChain (the `Attestation` account is a fixed-layout, Pinocchio-defined struct — ~150 lines).

### Account-size migration

Existing devnet tournaments will fail to deserialize after Tournament/Participant struct changes. Options:
- **Recommended:** redeploy program with a fresh program ID for Phase 1; treat devnet as a clean slate. MVP doc explicitly notes "Mainnet-prep migration to Squads 2-of-3 multisig is a Phase 7 submission gate" — i.e., MVP isn't on mainnet yet, so we don't owe state continuity.
- Alternative: ship a `realloc` ix and a one-time migration script. Adds complexity for no production benefit at this stage.

---

## SDK changes (`bracket-chain-sdk/`)

### Patterns to mirror
- Method file structure: `createTournament.ts` for the ix-builder pattern; `joinTournament.ts` for the params + return shape.
- PDA helpers: `pdas.ts` — `findTournamentPda`, `findVaultPda`, `findParticipantPda` precedent for `findSasAttestationPda`.
- Error mapping: `errors.ts` — typed `BracketChainSDKError` subclasses.
- IDL sync: manual `pnpm run sync-idl` after program build (`sync-idl.mjs` copies from `../bracket-chain-programs/target/idl/`).

### Files to modify

1. **`src/idl/bracket_chain.json` + `bracket_chain.ts`** — regenerate via `sync-idl` after program rebuild. Don't edit by hand.
2. **`src/types.ts`** — add `SupportedGame` and `SettlementMode` TS unions matching the Rust enums; add `game` and `settlementMode` to `Tournament` type; add `identityHash` and `identityAttestation` to `Participant` type.
3. **`src/pdas.ts`** — add:
   - `findSasCredentialPda(authority: PublicKey)` (mirror of SAS's seed pattern: `["credential", authority, name]`).
   - `findSasSchemaPda(credential: PublicKey, name: string)`.
   - `findSasAttestationPda(credential, schema, nonce: PublicKey)` — used to derive the expected attestation account for `joinTournament`.
4. **`src/errors.ts`** — add typed errors: `GameNotYetSupportedError`, `AttestationRequiredError`, `WrongAttestationCredentialError`, `AttestationExpiredError`, etc.
5. **`src/methods/createTournament.ts`** — extend params with `game?: SupportedGame` (default `Manual`) and `settlementMode?: SettlementMode` (default derived: `Oracle` when game ≠ Manual, else `Manual`).
6. **`src/methods/joinTournament.ts`** — accept optional `gameIdentityAttestation?: PublicKey`. When undefined and tournament.game ≠ Manual, derive it via `findSasAttestationPda` and the indexer's `getGameIdentity` lookup; throw `AttestationRequiredError` if the player has not yet linked.
7. **`src/methods/index.ts`** — export new `setSasConfig` method.
8. **`src/api.ts`** — `BracketChainIndexerClient` adds:
   - `getSteamLoginUrl(wallet: PublicKey): Promise<{ redirectUrl: string }>` — kicks off OpenID at the indexer.
   - `getGameIdentity(wallet: PublicKey, game: SupportedGame): Promise<{ attestationPda: PublicKey; identityHash: Uint8Array; expiry: number } | null>`.
9. **`src/index.ts`** — re-export new types, PDA helpers, errors, methods.

### New file

- **`src/methods/setSasConfig.ts`** — admin-only ix builder (mirrors `initialize_protocol.ts` if it exists; otherwise a one-shot script form).

### Dependencies to add (user-installed)

- `@solana-foundation/sas-sdk` (or whatever the official client is named) — for off-chain helpers to compute SAS PDAs in the SDK. If the user prefers to avoid the dep, hand-roll the seed derivation in `pdas.ts` (cheap, ~20 lines).

---

## Indexer changes (`bracket-chain-indexer/`)

This is the largest delta in the plan because Steam OpenID and SAS issuance are net-new product surface — there's no existing auth in the indexer to slot into.

### New module: `src/identity/`

- **`identity.module.ts`** — wires the controller + services; imported in `app.module.ts`.
- **`steam-openid.service.ts`** — handles Steam OpenID 2.0 (Steam still uses OpenID 2.0, not OAuth/OIDC). Recommended lib: `openid-client` (modern, maintained) or `passport-steam` (older, NestJS-friendly). Flow:
  1. `POST /identity/steam/login` with `{ wallet }` → builds the Steam OpenID redirect URL with `return_to` carrying the wallet as state.
  2. Steam redirects user back to `GET /identity/steam/callback`.
  3. Service verifies the OpenID response signature against Steam's IdP, extracts `steam_id_64`.
- **`sas-issuer.service.ts`** — wraps SAS SDK. Owns the indexer's signing keypair (loaded from `SAS_ISSUER_KEYPAIR` env var). On `onModuleInit`:
  - If `bracketchain-issuer` Credential PDA missing, calls SAS `create_credential` ix.
  - If `bracketchain.game_identity.v1` Schema PDA missing, calls SAS `create_schema` ix (one schema covers all games; the `game: u8` is a field inside the attestation data).
  - Logs the resulting Credential + Schema pubkeys at boot — the operator pastes these into the on-chain `set_sas_config` ix.
- **`identity.controller.ts`** — endpoints:
  - `POST /identity/steam/login` → returns `{ redirectUrl }`.
  - `GET /identity/steam/callback` → verifies, then calls `sas-issuer.createAttestation({ wallet, game: Dota2, steamId64, identityBytes })`, then redirects user back to the frontend deep link `bracketchain://identity-linked?game=dota2`.
  - `GET /identity/:wallet/:game` → returns the attestation PDA + parsed identity for the frontend's prefetch.

### Files to modify

1. **`src/app.module.ts`** — add `IdentityModule` to imports.
2. **`prisma/schema.prisma`** — add `GameIdentityAttestation` table:
   ```prisma
   model GameIdentityAttestation {
     id                 String   @id @default(cuid())
     wallet             String
     game               Int      // SupportedGame enum value
     attestationPda     String   @unique
     steamId64          String?
     identityHash       Bytes
     issuedAt           DateTime
     expiry             DateTime
     @@unique([wallet, game])
     @@index([wallet])
   }
   ```
3. **`src/webhooks/helius-parser.service.ts`** — extend event handlers:
   - `TournamentCreated` now carries `game` + `settlement_mode` — pass through to the DB row (requires Prisma schema bump on Tournament: `game: Int`, `settlementMode: Int`).
   - `ParticipantRegistered` now carries `identity_hash` — store on Participant row.
4. **`prisma/schema.prisma`** — add `game: Int @default(0)` and `settlementMode: Int @default(0)` to `Tournament`; add `identityHash: Bytes?` to `Participant`.
5. **`src/reconciliation/reconciliation.service.ts`** — extend the existing batch-fetch loop to surface `game` and `settlement_mode` on the Tournament during drift recovery. No new accounts to fetch in Phase 1 (no per-match oracle accounts yet).

### Dependencies (user-installed)

- `openid-client` (Steam OpenID 2.0)
- The SAS TypeScript client (whichever package the SAS repo ships as `clients/typescript`).
- `tweetnacl` for Ed25519 if the SAS client needs raw key handling (often already a transitive dep of @solana/web3.js).

### Environment vars

- `SAS_ISSUER_KEYPAIR` — base58 secret key for the indexer's SAS issuer identity. **Must be unique per environment.** Devnet key only — never reuse on mainnet.
- `STEAM_API_KEY` — for OpenID return-URL validation and (Phase 2) match-result lookups.
- `STEAM_RETURN_URL` — public callback URL of the indexer (e.g., `https://bracketchain-indexer-production.up.railway.app/identity/steam/callback`).
- `FRONTEND_DEEP_LINK_HOST` — for post-callback redirect.

---

## Frontend changes (`BracketChain-Frontend/`)

### Patterns to mirror
- Wizard step pattern in `features/tournament/steps/` — each step is a self-contained component; navigation in `CreateTournament.tsx`.
- Modal pattern in `features/tournament/view/CancelModal.tsx` for the Steam-connect UX.
- Indexer client singleton at `lib/sdk.ts` for the new `getGameIdentity` / `getSteamLoginUrl` calls.
- Toast pattern via Sonner already in place (`project_frontend_state_2026_05_10` memory).

### Files to modify

1. **`types/tournament.ts`** — add `game: SupportedGame`, `settlementMode: SettlementMode` to `Tournament`/`TournamentSummary`; add optional `gameIdentity?: { steamId64: string; identityHash: string }` to `Participant`.
2. **`lib/indexerToTournamentState.ts`** — map new indexer fields onto UI state.
3. **`features/tournament/create/CreateTournament.tsx`** — extend `DetailsData` with `game` and `settlementMode`. When `game` changes from `Manual` to anything else, auto-set `settlementMode = Oracle` (with a manual-override checkbox per the locked decision). Pass through to `createTournament` SDK call.
4. **`features/tournament/steps/DetailsStep.tsx`** — add a `GamePicker` field-group after the name input; show `SettlementModeBadge` (or just an info line) below it noting "Oracle settlement is enabled by default; full automation arrives in V1.2." This is the user-visible signal that Phase 1 stores intent, Phase 2 honors it.
5. **`features/tournament/steps/ValidateState.ts`** — add validation: `game ∈ enum`; if `game != Manual`, `settlementMode` must be set.
6. **`features/tournament/view/TournamentSidebar.tsx`** — Join button: when `tournament.game != Manual`, before allowing wallet click → SDK call, check `useGameIdentity(wallet, tournament.game)`. If absent, open `GameAuthModal` instead.
7. **`features/tournament/view/TournamentHeader.tsx`** — add `GameBadge` next to the status badge.

### New files

1. **`features/tournament/steps/GamePicker.tsx`** — select with the 5 supported games; in Phase 1 only `Manual` and `Dota 2` are selectable, others render as disabled with "Coming in V1.1".
2. **`features/tournament/GameAuthModal.tsx`** — fetches Steam login URL from indexer, opens in popup, listens for the deep-link callback (or polls indexer's `/identity/:wallet/:game` until present). Reuses `CancelModal`'s overlay pattern.
3. **`hooks/useGameIdentity.ts`** — TanStack Query hook against `indexer.getGameIdentity(wallet, game)`; staleTime ~5 min.
4. **`components/GameBadge.tsx`** — small reusable badge: game name + icon + colored variant.
5. **`constants/games.ts`** — `GAMES` enum-mirror + display metadata (label, icon, available-in-phase).

### Out of frontend scope for Phase 1
- Switchboard-feed-binding UI in ManageView.
- Dispute window countdown / `claimMatch` / `disputeMatch` buttons.
- Per-match oracle-vs-manual provenance badges in BracketView.

These are explicitly Phase 2 items; mentioning them here so the boundary is clear.

---

## SAS bootstrap (one-time, run by you on devnet)

After the program is redeployed and `set_sas_config` ix exists, run a single bootstrap script (suggested location: `bracket-chain-programs/scripts/bootstrap-sas.ts`):

1. Indexer boots, sees no `bracketchain-issuer` Credential, creates it via SAS `create_credential`. Logs the Credential pubkey.
2. Indexer creates the `bracketchain.game_identity.v1` Schema. Logs the Schema pubkey.
3. Operator calls BracketChain's `set_sas_config(credential, schemas[5])` with the indexer-logged pubkeys (schemas[0] = default zero pubkey for Manual; schemas[1] = the issued schema for Dota 2; schemas[2..4] = default until those games ship).
4. Verify via `solana account <credential>` and `solana account <schema>` on devnet.

Document this in `bracketchain-main/README.md` under a "V1.1 setup" section so a fresh deploy is reproducible.

---

## Verification (end-to-end devnet smoke)

1. **Program tests** (`bracket-chain-programs/tests/`):
   - Mocha test: create Manual tournament, join with no attestation → success (existing behavior unchanged).
   - Create Dota 2 tournament, join with no attestation → fails `AttestationRequired`.
   - Create attestation manually via SAS SDK, join with it → success; verify Participant.identity_hash matches keccak of first 32 attestation bytes.
   - Tamper with attestation (wrong nonce / wrong schema / expired) → each fails with the right error.
2. **Indexer integration test**: spin up against devnet, hit `POST /identity/steam/login` with a mock Steam IdP (or real Steam in manual smoke), follow the callback → verify `GameIdentityAttestation` row created and the attestation PDA is readable on-chain.
3. **Frontend smoke**: create Dota 2 tournament from the UI, attempt to join without connecting Steam → join button disabled with explanatory tooltip. Click "Connect Steam," complete OpenID, return to tournament page, join successfully.
4. **Backward compatibility**: existing Manual tournaments created against the prior program ID will be unreachable (different program ID after redeploy). Confirm the frontend gracefully ignores stale indexer rows (or run a one-time DB truncate on devnet — fine for a devnet-only deploy).
5. **MCP check**: re-run `Solana_Expert__Ask_For_Help` with the schema choices above to sanity-check no SAS gotchas (e.g., schema-paused handling, attestation expiry semantics) are missed.

---

## Open questions to resolve before kickoff

1. **SAS Rust crate availability** — does the SAS team ship a `no-entrypoint`-featured crate for in-program deserialization, or do we hand-roll the byte layout in `state/game.rs`? Affects ~150 LOC and the `Cargo.toml` delta.
2. **Schema versioning** — `bracketchain.game_identity.v1`: do we anticipate breaking the schema in Phase 2 (e.g., adding Riot PUUID alongside Steam ID), or keep it forward-compatible from day one? Recommend a single schema with a `kind: u8` discriminator inside `data` so we never need to migrate.
3. **Devnet upgrade authority** — `lib.rs:12` declares program ID `AuXJK…F1`. Plan assumes a fresh program ID for clean Phase 1 redeploy. Confirm OK to abandon the current devnet program ID (it's not on mainnet, and the MVP memo says "Mainnet-prep migration to Squads 2-of-3 multisig is a Phase 7 submission gate" — so no continuity owed).
4. **Steam OpenID popup vs. full-page redirect** — frontend UX choice. Popup is friendlier but blocked by some browsers; full-page redirect requires preserving in-progress tournament state in localStorage. Plan defaults to popup for V1.1; flag for revisit if it causes Phantom/Solflare race conditions.

---

## Explicitly out of scope (Phase 2+)

These are *not* in this plan and will require follow-up plans:

- Switchboard `@switchboard-xyz/on-demand` integration.
- New ixs: `bind_match_feed`, `report_result_oracle`, `claim_match`, `dispute_match`.
- Per-match `MatchNode` additions: `external_match_id`, `switchboard_feed`, `oracle_reported_at`, `disputed`.
- Dispute window timer + `arbitrator: Pubkey` on Tournament.
- Multi-source aggregation inside feed jobs (Steam + OpenDota + STRATZ).
- CS2 via FACEIT, Valorant/LoL via Riot, PUBG via official API.
- Frontend: feed-binding modal in ManageView, dispute-window countdown badge, `claimMatch`/`disputeMatch` buttons, oracle-vs-manual provenance display.
- Squads multisig as arbitrator.
- BR placement-payout extensions to the canonical `MatchOutcome` envelope.

Recording these here so the Phase 2 plan can pick up cleanly without re-deriving the boundary.

---

## Critical files (quick reference)

**Program:**
- `bracket-chain-programs/programs/bracket-chain/src/lib.rs:12-66` — program entrypoints (add `set_sas_config`, extend `create_tournament` + `join_tournament` params)
- `bracket-chain-programs/programs/bracket-chain/src/state/tournament.rs:43-79` — add `game`, `settlement_mode`
- `bracket-chain-programs/programs/bracket-chain/src/state/participant.rs:1-11` — add `identity_hash`, `identity_attestation`
- `bracket-chain-programs/programs/bracket-chain/src/state/protocol_config.rs` — add `sas_credential`, `sas_schemas[5]`
- `bracket-chain-programs/programs/bracket-chain/src/instructions/join_tournament.rs:9-52` — insert attestation validation block
- `bracket-chain-programs/programs/bracket-chain/src/instructions/create_tournament.rs` — extend params, validate game ≠ unsupported
- `bracket-chain-programs/programs/bracket-chain/src/state/game.rs` — new
- `bracket-chain-programs/programs/bracket-chain/src/instructions/set_sas_config.rs` — new
- `bracket-chain-programs/programs/bracket-chain/src/errors.rs` — add 8 variants
- `bracket-chain-programs/programs/bracket-chain/src/constants.rs` — add `SAS_PROGRAM_ID`
- `bracket-chain-programs/programs/bracket-chain/src/events.rs` — extend `TournamentCreated`, `ParticipantRegistered`

**SDK:**
- `bracket-chain-sdk/src/types.ts`, `pdas.ts`, `errors.ts`, `index.ts`, `api.ts` — extensions
- `bracket-chain-sdk/src/methods/createTournament.ts`, `joinTournament.ts` — extend
- `bracket-chain-sdk/src/methods/setSasConfig.ts` — new
- `bracket-chain-sdk/scripts/sync-idl.mjs` — run after program rebuild

**Indexer:**
- `bracket-chain-indexer/src/app.module.ts` — wire `IdentityModule`
- `bracket-chain-indexer/src/identity/` — new module (3 services + controller + module)
- `bracket-chain-indexer/src/webhooks/helius-parser.service.ts` — extend `TournamentCreated`, `ParticipantRegistered` handling
- `bracket-chain-indexer/prisma/schema.prisma` — add `GameIdentityAttestation`, extend `Tournament` + `Participant`
- `bracket-chain-indexer/src/reconciliation/reconciliation.service.ts` — surface new fields

**Frontend:**
- `BracketChain-Frontend/types/tournament.ts` — extend
- `BracketChain-Frontend/lib/indexerToTournamentState.ts` — map new fields
- `BracketChain-Frontend/features/tournament/create/CreateTournament.tsx` — wire game + settlement
- `BracketChain-Frontend/features/tournament/steps/DetailsStep.tsx`, `ValidateState.ts` — extend
- `BracketChain-Frontend/features/tournament/steps/GamePicker.tsx` — new
- `BracketChain-Frontend/features/tournament/GameAuthModal.tsx` — new
- `BracketChain-Frontend/features/tournament/view/TournamentSidebar.tsx`, `TournamentHeader.tsx` — extend
- `BracketChain-Frontend/hooks/useGameIdentity.ts` — new
- `BracketChain-Frontend/components/GameBadge.tsx` — new
- `BracketChain-Frontend/constants/games.ts` — new

**Docs:**
- `bracketchain-main/README.md` — V1.1 setup section + MVP→V1.1 delta.
- `bracketchain-mvp-plan.md` — add a Phase 8 / V1.1 section pointing at this plan; keep MVP locked notes unchanged.
