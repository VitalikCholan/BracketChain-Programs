use anchor_lang::prelude::*;

/// Who authored the result currently sitting in a `MatchNode`'s proposal
/// envelope. The settlement flow treats every source uniformly — a proposal is
/// a proposal regardless of origin (C2: the Oracle is "just another proposer",
/// no parallel dispute system). Explicit discriminants are part of the wire
/// contract: the indexer and SDK decode by number.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, Debug, InitSpace)]
pub enum ProposalSource {
    /// No proposal recorded yet — the envelope is empty.
    None = 0,
    /// Proposed by one of the two match players (`propose_result`).
    Player = 1,
    /// Written by the Switchboard On-Demand oracle (Stage C / V1.2 Dota 2).
    Oracle = 2,
    /// Reserved for a future trusted game-server reporter (post-V1).
    GameServer = 3,
}
