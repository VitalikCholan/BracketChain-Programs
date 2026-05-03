use anchor_lang::prelude::*;

#[event]
pub struct TournamentCreated {
    pub tournament: Pubkey,
    pub organizer: Pubkey,
    pub token_mint: Pubkey,
    pub entry_fee: u64,
    pub organizer_deposit: u64,
    pub max_participants: u16,
    pub payout_preset: u8,
    pub registration_deadline: i64,
}

#[event]
pub struct ParticipantRegistered {
    pub tournament: Pubkey,
    pub wallet: Pubkey,
    pub participant_index: u16,
}

#[event]
pub struct TournamentStarted {
    pub tournament: Pubkey,
    pub bracket_size: u16,
    pub participant_count: u16,
    pub seed_hash: [u8; 32],
    pub started_at: i64,
}

#[event]
pub struct MatchReported {
    pub tournament: Pubkey,
    pub round: u8,
    pub match_index: u16,
    pub winner: Pubkey,
    pub reported_at: i64,
}

#[event]
pub struct TournamentCompleted {
    pub tournament: Pubkey,
    pub champion: Pubkey,
    pub gross_pool: u64,
    pub fee_amount: u64,
    pub net_pool: u64,
    pub completed_at: i64,
    /// Per-placement breakdown: place=1..=N for prize tiers (champion is place=1).
    /// Includes only non-zero payouts in CPI-execution order.
    pub placement_payouts: Vec<PlacementPayout>,
    /// Treasury wallet receiving the protocol fee.
    /// Self-contained event — indexers don't need extra reads.
    pub treasury_recipient: Pubkey,
}

#[derive(Clone, AnchorSerialize, AnchorDeserialize)]
pub struct PlacementPayout {
    pub place: u8,
    pub recipient: Pubkey,
    pub amount: u64,
}

#[event]
pub struct TournamentCancelled {
    pub tournament: Pubkey,
    pub authority: Pubkey,
    pub cancelled_at: i64,
}

#[event]
pub struct RefundIssued {
    pub tournament: Pubkey,
    pub wallet: Pubkey,
    pub amount: u64,
}
