use anchor_lang::prelude::*;

/// Pre-match commitment binding an on-chain match to a real game lobby (Stage C
/// / V1.2 Oracle settlement). Written by `commit_match_lobby` **before** the
/// lobby launches, so the oracle cannot be redirected post-hoc: the Switchboard
/// `OracleJob` is parameterized with `lobby_id` + both `player_*_game_id`, and
/// `propose_result_oracle` verifies the feed's winning id hashes to one of the
/// two committed ids.
///
/// `player_*_game_id` are copied verbatim from each `Participant.identity_hash`
/// — a 32-byte `SHA-256(steam_id_64 as u64 little-endian)` fingerprint (set by
/// the indexer's SAS issuer in V1.1, A-9). The OracleJob must reproduce that
/// exact hash, or every proposal fails `OracleWinnerNotInMatch`.
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
    pub committed_at: i64,
    pub committed_slot: u64,
}
