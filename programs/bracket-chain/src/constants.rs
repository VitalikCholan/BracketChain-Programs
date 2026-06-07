use anchor_lang::prelude::*;

/// Wire version stamped as the first field of every `#[event]` struct (C10).
/// The indexer parser rejects events whose `event_version` differs, preventing
/// silent Borsh mis-decode when the event layout changes in a future redeploy.
#[constant]
pub const EVENT_VERSION_V1: u8 = 1;

#[constant]
pub const PROTOCOL_FEE_BPS: u16 = 350;

#[constant]
pub const BPS_DENOMINATOR: u16 = 10_000;

#[constant]
pub const MIN_PARTICIPANTS: u16 = 2;

#[constant]
pub const MAX_PARTICIPANTS: u16 = 128;

/// Grace period after a dispute before `force_claim_disputed` lets anyone
/// finalize the proposed winner — the trustless backstop against an organizer
/// who never calls `resolve_dispute`. 24 hours.
#[constant]
pub const FORCE_CLAIM_WINDOW_SECS: i64 = 86_400;

pub const MAX_TOURNAMENT_NAME_LEN: usize = 32;

pub const PROTOCOL_CONFIG_SEED: &[u8] = b"protocol_config";
pub const TOURNAMENT_SEED: &[u8] = b"tournament";
pub const VAULT_SEED: &[u8] = b"vault";
pub const PARTICIPANT_SEED: &[u8] = b"participant";
pub const MATCH_SEED: &[u8] = b"match";

/// Solana Attestation Service program ID (same on devnet + mainnet). Incoming
/// game-identity attestations at `join_tournament` must be owned by this.
pub const SAS_PROGRAM_ID: Pubkey =
    pubkey!("22zoJMtdu4tQc2PzL74ZUT7FrwgB1Udec8DdW4yw4BdG");

/// Switchboard On-Demand program IDs (the owner of a `RandomnessAccountData`).
/// `reveal_seed` / `request_seed` accept either so the same program binary works
/// on devnet (V1 target) and mainnet. The crate's `is_devnet()` relies on an env
/// var / build cfg that is not set inside the SBF runtime, so we check ownership
/// against these explicit constants instead.
pub const SWITCHBOARD_ON_DEMAND_DEVNET: Pubkey =
    pubkey!("Aio4gaXjXzJNVLtzwtNVmSqGKpANtXhybbkhtAC94ji2");
pub const SWITCHBOARD_ON_DEMAND_MAINNET: Pubkey =
    pubkey!("SBondMDrcV3K4kxZR1HNVT7osZxAHVHgYXL5Ze1oMUv");

/// Max payout placement slots. Custom presets carry exactly this many; the
/// fixed presets are zero-padded to it. Widened 7 → 8 in Stage D (D-1) so a
/// `Custom([u16;8])` can express up to 8 placements (gate G6).
pub const MAX_PAYOUT_SLOTS: usize = 8;

pub const PAYOUT_WTA: [u16; MAX_PAYOUT_SLOTS] = [10_000, 0, 0, 0, 0, 0, 0, 0];
pub const PAYOUT_STANDARD: [u16; MAX_PAYOUT_SLOTS] = [6_000, 2_500, 1_500, 0, 0, 0, 0, 0];
pub const PAYOUT_DEEP: [u16; MAX_PAYOUT_SLOTS] = [4_000, 2_500, 1_500, 1_000, 500, 300, 200, 0];

/// Bounds for `set_oracle_config` (L-2 hardening). `min_oracle_samples` must be
/// at least this — `min_oracle_samples = 0` would let
/// `PullFeedAccountData::get_value` settle on a *single* submission (its
/// `submissions.len() < 0` guard never trips), silently degrading the oracle
/// trust threshold to one sample.
pub const MIN_ORACLE_SAMPLES_FLOOR: u32 = 1;
/// Ceiling on `max_stale_slots`: the oldest an oracle sample may be and still
/// feed a settlement. ~9000 slots ≈ 1 hour at ~2.5 slots/s — generous for any
/// real feed cadence while preventing an admin from accepting arbitrarily stale
/// data (and keeping `clock_slot - max_staleness` well clear of underflow).
pub const MAX_ORACLE_STALE_SLOTS_CEILING: u32 = 9_000;
