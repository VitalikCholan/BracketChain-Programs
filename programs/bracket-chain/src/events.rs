use anchor_lang::prelude::*;

#[event]
pub struct TournamentCreated {
    /// Event wire version (C10). Always first field. See `EVENT_VERSION_V1`.
    pub event_version: u8,
    pub tournament: Pubkey,
    pub organizer: Pubkey,
    pub token_mint: Pubkey,
    pub entry_fee: u64,
    pub organizer_deposit: u64,
    pub max_participants: u16,
    pub payout_preset: u8,
    pub registration_deadline: i64,
    /// Human-readable tournament name (≤ MAX_TOURNAMENT_NAME_LEN bytes).
    /// Indexers consume this to populate listing UIs without a follow-up RPC.
    pub name: String,
}

#[event]
pub struct ParticipantRegistered {
    pub event_version: u8,
    pub tournament: Pubkey,
    pub wallet: Pubkey,
    pub participant_index: u16,
}

#[event]
pub struct TournamentStarted {
    pub event_version: u8,
    pub tournament: Pubkey,
    pub bracket_size: u16,
    pub participant_count: u16,
    pub seed_hash: [u8; 32],
    pub started_at: i64,
}

#[event]
pub struct MatchReported {
    pub event_version: u8,
    pub tournament: Pubkey,
    /// Bracket lane (C9). `0` for single-elimination (V1). The canonical
    /// "advance the bracket" signal — emitted by every finalize path.
    pub bracket: u8,
    pub round: u8,
    pub match_index: u16,
    pub winner: Pubkey,
    pub reported_at: i64,
}

#[event]
pub struct TournamentCompleted {
    pub event_version: u8,
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
    pub event_version: u8,
    pub tournament: Pubkey,
    pub authority: Pubkey,
    pub cancelled_at: i64,
}

#[event]
pub struct RefundIssued {
    pub event_version: u8,
    pub tournament: Pubkey,
    pub wallet: Pubkey,
    pub amount: u64,
}

// ── Player-reported / Oracle settlement envelope events (Stage B) ───────────
// `MatchReported` remains the canonical "this match is final, advance the
// bracket" signal emitted by every finalize path. The events below are the
// notification-kernel granularity layered on top (B-14): who proposed, who
// disputed, and how a pending result ultimately closed.

#[event]
pub struct ResultProposed {
    pub event_version: u8,
    pub tournament: Pubkey,
    pub bracket: u8,
    pub round: u8,
    pub match_index: u16,
    /// `ProposalSource` discriminant (1 = Player, 2 = Oracle, …).
    pub source: u8,
    pub proposer: Pubkey,
    pub proposed_winner: Pubkey,
    /// Deadline after which `claim_result` may permissionlessly finalize.
    pub claim_deadline: i64,
    pub proposed_at: i64,
}

#[event]
pub struct ResultDisputed {
    pub event_version: u8,
    pub tournament: Pubkey,
    pub bracket: u8,
    pub round: u8,
    pub match_index: u16,
    pub disputer: Pubkey,
    pub dispute_reason: u8,
    /// Re-armed deadline after which `force_claim_disputed` may finalize.
    pub force_claim_deadline: i64,
    pub disputed_at: i64,
}

#[event]
pub struct ResultClaimed {
    pub event_version: u8,
    pub tournament: Pubkey,
    pub bracket: u8,
    pub round: u8,
    pub match_index: u16,
    pub winner: Pubkey,
    /// `true` when finalized via `force_claim_disputed` (post-dispute backstop)
    /// rather than the ordinary undisputed `claim_result`.
    pub forced: bool,
    pub claimed_at: i64,
}

#[event]
pub struct DisputeResolved {
    pub event_version: u8,
    pub tournament: Pubkey,
    pub bracket: u8,
    pub round: u8,
    pub match_index: u16,
    pub arbitrator: Pubkey,
    pub winner: Pubkey,
    pub resolved_at: i64,
}

// ── V1.2 Oracle settlement (Stage C) ────────────────────────────────────────
// Only the commit/bind ceremony needs new events. The oracle *result* reuses
// V1's `ResultProposed` (with `source = Oracle`); claim/dispute/resolve fire
// V1's existing events unchanged — the indexer differentiates by `source`.

#[event]
pub struct MatchLobbyCommitted {
    pub event_version: u8,
    pub tournament: Pubkey,
    pub bracket: u8,
    pub round: u8,
    pub match_index: u16,
    /// Organizer-chosen pre-match lobby identifier (16 bytes).
    pub lobby_id: [u8; 16],
    pub committed_at: i64,
}

#[event]
pub struct MatchFeedBound {
    pub event_version: u8,
    pub tournament: Pubkey,
    pub bracket: u8,
    pub round: u8,
    pub match_index: u16,
    /// Switchboard On-Demand `PullFeedAccountData` PDA bound to this match.
    pub switchboard_feed: Pubkey,
}
