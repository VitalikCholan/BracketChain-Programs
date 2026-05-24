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

pub const PAYOUT_WTA: [u16; 7] = [10_000, 0, 0, 0, 0, 0, 0];
pub const PAYOUT_STANDARD: [u16; 7] = [6_000, 2_500, 1_500, 0, 0, 0, 0];
pub const PAYOUT_DEEP: [u16; 7] = [4_000, 2_500, 1_500, 1_000, 500, 300, 200];
