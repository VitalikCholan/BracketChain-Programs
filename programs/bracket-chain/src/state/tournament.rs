use anchor_lang::prelude::*;

use crate::constants::MAX_TOURNAMENT_NAME_LEN;

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, Debug, InitSpace)]
pub enum TournamentStatus {
    Registration,
    PendingBracketInit,
    Active,
    Completed,
    Cancelled,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, Debug, InitSpace)]
pub enum PayoutPreset {
    WinnerTakesAll,
    Standard,
    Deep,
}

impl PayoutPreset {
    pub fn min_participants(&self) -> u16 {
        match self {
            PayoutPreset::WinnerTakesAll => 1,
            PayoutPreset::Standard => 3,
            PayoutPreset::Deep => 7,
        }
    }

    pub fn basis_points(&self) -> [u16; 7] {
        match self {
            PayoutPreset::WinnerTakesAll => crate::constants::PAYOUT_WTA,
            PayoutPreset::Standard => crate::constants::PAYOUT_STANDARD,
            PayoutPreset::Deep => crate::constants::PAYOUT_DEEP,
        }
    }

    pub fn placement_count(&self) -> usize {
        self.basis_points().iter().filter(|bps| **bps > 0).count()
    }
}

#[account]
#[derive(InitSpace)]
pub struct Tournament {
    pub organizer: Pubkey,
    #[max_len(MAX_TOURNAMENT_NAME_LEN)]
    pub name: String,
    /// SPL Token mint for the prize pool. Any mint allowed (USDC, wSOL for
    /// SOL tournaments, custom). Frontend gatekeeps user-facing selection.
    pub token_mint: Pubkey,
    pub vault: Pubkey,
    pub entry_fee: u64,
    /// Optional organizer top-up to the prize pool, transferred into the vault
    /// at creation. `0` is allowed. Refunded back to the organizer if the
    /// tournament is cancelled before the first match. On completion, it stays
    /// in the vault and is distributed as part of the prize pool (Variant B).
    pub organizer_deposit: u64,
    /// Tracks whether the organizer's deposit refund has been issued during a
    /// cancellation. Independent of per-participant `refund_paid` flags so the
    /// two paths can be processed in any order across cancel chunks.
    pub organizer_deposit_refunded: bool,
    pub max_participants: u16,
    pub bracket_size: u16,
    pub participant_count: u16,
    pub matches_initialized: u16,
    pub matches_reported: u16,
    pub total_matches: u16,
    pub registration_deadline: i64,
    pub created_at: i64,
    pub started_at: i64,
    pub completed_at: i64,
    pub status: TournamentStatus,
    pub payout_preset: PayoutPreset,
    pub seed_hash: [u8; 32],
    pub champion: Pubkey,
    pub bump: u8,
    pub vault_bump: u8,
}
