use anchor_lang::prelude::*;

/// Pre-match commitment binding an on-chain match to a real game lobby (Stage C
/// / V1.2 Oracle settlement). Written by `commit_match_lobby` **before** the
/// lobby launches.
///
/// **Anti-redirection (trust model).** The Switchboard `OracleJob` is
/// parameterized with `lobby_id` + both `player_*_game_id`, so those identities
/// are baked into the feed's `feed_hash` (SHA-256 of the job schema). We commit
/// the off-chain-computed `expected_feed_hash` here, and `bind_match_feed`
/// requires the bound feed's `feed_hash` to equal it — cryptographically tying
/// the feed to *these two identities in this lobby*. The job then returns only
/// the **winner index** (0 = player_a, 1 = player_b); identity is verified via
/// the feed binding, not via the (range-limited) feed value. The dispute window
/// + arbitrator is the ultimate backstop (the oracle is "just another
/// proposer"). `feed.authority` pinning (Layer 2) is deferred.
///
/// `player_*_game_id` are copied verbatim from each `Participant.identity_hash`
/// (32-byte `SHA-256(steam_id_64 LE)`, A-9): they are the OracleJob query
/// params (and the basis for `expected_feed_hash`) plus an audit record.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, Debug, InitSpace)]
pub struct MatchCommitment {
    /// Organizer-chosen pre-match identifier (frontend-generated random 16
    /// bytes). Need not be the real Dota lobby id — only unforgeable post-hoc;
    /// pasted into the lobby name as the human-side ceremony.
    pub lobby_id: [u8; 16],
    /// `participant_a.identity_hash` at commit time.
    pub player_a_game_id: [u8; 32],
    /// `participant_b.identity_hash` at commit time.
    pub player_b_game_id: [u8; 32],
    /// SHA-256 of the OracleJob schema the feed-factory will run for this match
    /// (computed off-chain from `lobby_id` + both `player_*_game_id`).
    /// `bind_match_feed` enforces `feed.feed_hash == expected_feed_hash`.
    pub expected_feed_hash: [u8; 32],
    pub committed_at: i64,
    pub committed_slot: u64,
}
