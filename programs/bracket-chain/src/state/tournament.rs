use anchor_lang::prelude::*;

use super::game::{SettlementMode, SupportedGame};
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
    /// `0` is allowed. Treated as a refundable commitment (Variant A):
    /// returned to the organizer on `cancel_tournament` (pre-start) AND on
    /// `report_result` final-match. The deposit is excluded from the
    /// prize-pool basis — protocol fee + placement payouts apply to
    /// `vault.amount - organizer_deposit` only.
    pub organizer_deposit: u64,
    /// Set true once the deposit has been refunded — by `cancel_tournament`
    /// (any-call, idempotent across chunks) or by `report_result` final-match.
    /// Independent of per-participant `refund_paid` flags.
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
    // ── V1.1 additions (appended — positional Borsh; never reorder above) ──
    /// Game played; gates SAS identity requirement at `join_tournament`.
    pub game: SupportedGame,
    /// Who may report results. Locked at create-time.
    pub settlement_mode: SettlementMode,
    /// Dispute window (seconds) for PlayerReported / Oracle settlement. Unused
    /// by OrganizerOnly. Wired by the V1 player-reported stage of this redeploy.
    pub dispute_window_secs: u32,
    /// Switchboard randomness account committed via `request_seed` (VRF stage).
    pub vrf_randomness_account: Pubkey,
    /// Slot the VRF commitment was made at; `reveal_seed` reads after it passes.
    pub vrf_commit_slot: u64,
    /// True once `reveal_seed` has populated `seed_hash` from VRF. Gates
    /// `start_tournament` for non-OrganizerOnly tournaments.
    pub seed_revealed: bool,
}
