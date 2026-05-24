use anchor_lang::prelude::*;

use super::proposal_source::ProposalSource;

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, Debug, InitSpace)]
pub enum MatchStatus {
    Pending,
    Active,
    Completed,
}

#[account]
#[derive(InitSpace)]
pub struct MatchNode {
    pub tournament: Pubkey,
    /// Bracket lane this node lives in. `0` for single-elimination (V1). Part
    /// of the PDA seed (C9 schema-prep): future formats add a losers' bracket
    /// (`1`) or per-group lanes without a second redeploy. Always `0` today.
    pub bracket: u8,
    pub round: u8,
    pub match_index: u16,
    pub player_a: Pubkey,
    pub player_b: Pubkey,
    pub winner: Pubkey,
    pub status: MatchStatus,
    pub bye: bool,
    pub bump: u8,
    // ── Proposal envelope (V1 player-reported / Oracle settlement) ──────────
    // Populated by `propose_result` (or the oracle). Finalized by
    // `confirm_result` / `claim_result` / `resolve_dispute`. For OrganizerOnly
    // tournaments these stay at their defaults — `report_result` writes the
    // winner directly without ever touching the envelope.
    /// Origin of the pending proposal. `None` ⇒ envelope empty.
    pub proposal_source: ProposalSource,
    /// Wallet that authored the pending proposal (a match player, or the
    /// oracle's reporter key). `Pubkey::default()` when empty.
    pub proposer: Pubkey,
    /// Winner asserted by the pending proposal.
    pub proposed_winner: Pubkey,
    /// Unix time the proposal was recorded.
    pub proposed_at: i64,
    /// Unix time after which a permissionless finalize is allowed. Set to
    /// `proposed_at + dispute_window_secs` on propose; **re-armed** to
    /// `now + FORCE_CLAIM_WINDOW_SECS` (24h) on dispute. The `disputed` flag
    /// selects which permissionless ix may act past it: `claim_result` while
    /// `!disputed`, `force_claim_disputed` while `disputed` (organizer silence
    /// backstop). `resolve_dispute` ignores it — the organizer may act anytime.
    pub claim_deadline: i64,
    /// Set by `dispute_result`; blocks `claim_result` and routes the match to
    /// the organizer arbitrator (`resolve_dispute`), with a 24h
    /// `force_claim_disputed` backstop against organizer silence.
    pub disputed: bool,
    /// Free-form reason code supplied by the disputer (`0` = unspecified).
    /// Surfaced by the indexer's notification kernel; not interpreted on-chain.
    pub dispute_reason: u8,
}
