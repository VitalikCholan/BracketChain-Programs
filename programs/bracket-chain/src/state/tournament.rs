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
    /// Optional organizer top-up transferred into the vault at creation.
    /// `0` is allowed. Treated as a sponsored prize contribution (Variant B):
    /// stays in the vault on completion and is distributed as part of the
    /// prize-pool basis (`gross_pool = vault.amount`, including this deposit).
    /// Refunded only on pre-start `cancel_tournament`.
    pub organizer_deposit: u64,
    /// Set true once the deposit has been refunded by `cancel_tournament`
    /// (any-call, idempotent across chunks). Only meaningful in the
    /// `Cancelled` path — on `Completed`, the deposit goes to placements,
    /// not back to the organizer, and this flag stays `false`.
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
