use anchor_lang::prelude::*;

#[constant]
pub const PROTOCOL_FEE_BPS: u16 = 350;

#[constant]
pub const BPS_DENOMINATOR: u16 = 10_000;

#[constant]
pub const MIN_PARTICIPANTS: u16 = 2;

#[constant]
pub const MAX_PARTICIPANTS: u16 = 128;

pub const MAX_TOURNAMENT_NAME_LEN: usize = 64;

pub const PROTOCOL_CONFIG_SEED: &[u8] = b"protocol_config";
pub const TOURNAMENT_SEED: &[u8] = b"tournament";
pub const VAULT_SEED: &[u8] = b"vault";
pub const PARTICIPANT_SEED: &[u8] = b"participant";
pub const MATCH_SEED: &[u8] = b"match";

pub const PAYOUT_WTA: [u16; 7] = [10_000, 0, 0, 0, 0, 0, 0];
pub const PAYOUT_STANDARD: [u16; 7] = [6_000, 2_500, 1_500, 0, 0, 0, 0];
pub const PAYOUT_DEEP: [u16; 7] = [4_000, 2_500, 1_500, 1_000, 500, 300, 200];
