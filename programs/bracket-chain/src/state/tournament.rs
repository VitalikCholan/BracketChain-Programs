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
    /// Mid-tournament cancellation (Stage E). Distinct from `Cancelled`
    /// (pre-start) for analytics/UI. Terminal — no reactivation. Disc = 5;
    /// appended, never reorder (wire contract).
    PartialCancelled,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, Debug, InitSpace)]
pub enum PayoutPreset {
    WinnerTakesAll,
    Standard,
    Deep,
    /// Organizer-defined basis-point split over up to `MAX_PAYOUT_SLOTS` (8)
    /// placements. Validated at `create_tournament` (sum == 10_000, no gaps,
    /// `slots[0] > 0`, `placement_count <= max_participants`). Stage D (D-1).
    Custom([u16; crate::constants::MAX_PAYOUT_SLOTS]),
}

impl PayoutPreset {
    pub fn min_participants(&self) -> u16 {
        match self {
            PayoutPreset::WinnerTakesAll => 1,
            PayoutPreset::Standard => 3,
            PayoutPreset::Deep => 7,
            // A custom split needs at least as many entrants as funded slots.
            PayoutPreset::Custom(_) => self.placement_count() as u16,
        }
    }

    pub fn basis_points(&self) -> [u16; crate::constants::MAX_PAYOUT_SLOTS] {
        match self {
            PayoutPreset::WinnerTakesAll => crate::constants::PAYOUT_WTA,
            PayoutPreset::Standard => crate::constants::PAYOUT_STANDARD,
            PayoutPreset::Deep => crate::constants::PAYOUT_DEEP,
            PayoutPreset::Custom(slots) => *slots,
        }
    }

    pub fn placement_count(&self) -> usize {
        self.basis_points().iter().filter(|bps| **bps > 0).count()
    }

    /// Validate a `Custom` split. No-op for the fixed presets (always valid).
    /// Rules: every funded slot is contiguous from index 0 (no gaps), the
    /// winner slot is funded, and the bps sum to exactly `BPS_DENOMINATOR`.
    /// `placement_count <= max_participants` is checked by the caller (it owns
    /// `max_participants`). Stage D (D-1).
    pub fn validate_custom(&self) -> Result<()> {
        let slots = match self {
            PayoutPreset::Custom(slots) => slots,
            _ => return Ok(()),
        };
        require!(slots[0] > 0, crate::errors::BracketChainError::InvalidCustomPayout);
        // No gaps: once a zero appears, every later slot must also be zero.
        let mut ended = false;
        let mut sum: u32 = 0;
        for &bps in slots.iter() {
            if bps == 0 {
                ended = true;
            } else {
                require!(!ended, crate::errors::BracketChainError::InvalidCustomPayout);
                sum += bps as u32;
            }
        }
        require!(
            sum == crate::constants::BPS_DENOMINATOR as u32,
            crate::errors::BracketChainError::InvalidCustomPayout
        );
        Ok(())
    }
}

#[cfg(test)]
mod payout_tests {
    use super::*;

    fn custom(slots: [u16; 8]) -> PayoutPreset {
        PayoutPreset::Custom(slots)
    }

    #[test]
    fn valid_custom_splits_pass() {
        // G6 split (50/30/20 → bps), and a full 8-deep split.
        assert!(custom([5000, 3000, 2000, 0, 0, 0, 0, 0]).validate_custom().is_ok());
        assert!(custom([2500, 2000, 1500, 1500, 1000, 800, 500, 200])
            .validate_custom()
            .is_ok());
        assert!(custom([10000, 0, 0, 0, 0, 0, 0, 0]).validate_custom().is_ok());
    }

    #[test]
    fn sum_not_10000_is_rejected() {
        assert!(custom([5000, 3000, 1000, 0, 0, 0, 0, 0]).validate_custom().is_err());
        assert!(custom([6000, 3000, 2000, 0, 0, 0, 0, 0]).validate_custom().is_err());
    }

    #[test]
    fn gap_between_funded_slots_is_rejected() {
        assert!(custom([5000, 0, 5000, 0, 0, 0, 0, 0]).validate_custom().is_err());
    }

    #[test]
    fn unfunded_winner_is_rejected() {
        assert!(custom([0, 6000, 4000, 0, 0, 0, 0, 0]).validate_custom().is_err());
    }

    #[test]
    fn fixed_presets_skip_custom_validation() {
        assert!(PayoutPreset::WinnerTakesAll.validate_custom().is_ok());
        assert!(PayoutPreset::Standard.validate_custom().is_ok());
        assert!(PayoutPreset::Deep.validate_custom().is_ok());
    }

    #[test]
    fn placement_count_and_min_participants() {
        let p = custom([5000, 3000, 2000, 0, 0, 0, 0, 0]);
        assert_eq!(p.placement_count(), 3);
        assert_eq!(p.min_participants(), 3);
        // Fixed presets keep their declared minimums.
        assert_eq!(PayoutPreset::WinnerTakesAll.min_participants(), 1);
        assert_eq!(PayoutPreset::Deep.placement_count(), 7);
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
    /// `0` is allowed. Treated as a sponsored prize contribution (Variant B,
    /// R13 ratified 2026-06-05): stays in the vault on completion and is
    /// distributed as part of the prize-pool basis (`gross_pool =
    /// vault.amount`, including this deposit; the protocol fee applies to it).
    /// Refunded only on the cancel paths — pre-start `cancel_tournament` and
    /// mid-tournament `partial_refund_chunk`.
    pub organizer_deposit: u64,
    /// Set true once the deposit has been refunded by `cancel_tournament` or
    /// `partial_refund_chunk` (any-call, idempotent across chunks). Only
    /// meaningful on the `Cancelled` / `PartialCancelled` paths — on
    /// `Completed`, the deposit goes to placements, not back to the
    /// organizer, and this flag stays `false`.
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
    // ── V1.2 Oracle settlement (Stage C; appended — never reorder above) ────
    /// May dispute an Oracle proposal and call `resolve_dispute`. Defaults to
    /// `organizer` at create-time (Squads multisig reassignment is V1.3).
    pub arbitrator: Pubkey,
}
