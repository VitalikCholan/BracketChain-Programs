use anchor_lang::prelude::*;

use crate::constants::{EVENT_VERSION_V1, MATCH_SEED, PARTICIPANT_SEED, TOURNAMENT_SEED};
use crate::errors::BracketChainError;
use crate::events::MatchLobbyCommitted;
use crate::state::{
    MatchCommitment, MatchNode, MatchStatus, Participant, SettlementMode, Tournament,
    TournamentStatus,
};

/// Stage C / V1.2: the organizer commits a match to a real game lobby **before**
/// it launches, snapshotting both players' `identity_hash` into a
/// `MatchCommitment`. This is the anti-redirection anchor — the Switchboard feed
/// bound later (`bind_match_feed`) and read by `propose_result_oracle` must
/// resolve to one of these committed identities.
///
/// The match is identified by the `match_account` PDA itself (self-referential
/// seeds, same pattern as `propose_result`); only `lobby_id` is an argument.
#[derive(Accounts)]
pub struct CommitMatchLobby<'info> {
    #[account(address = tournament.organizer @ BracketChainError::UnauthorizedAuthority)]
    pub organizer: Signer<'info>,

    #[account(
        seeds = [
            TOURNAMENT_SEED,
            tournament.organizer.as_ref(),
            tournament.name.as_bytes(),
        ],
        bump = tournament.bump,
    )]
    pub tournament: Box<Account<'info, Tournament>>,

    #[account(
        mut,
        seeds = [
            MATCH_SEED,
            tournament.key().as_ref(),
            &[match_account.bracket],
            &[match_account.round],
            &match_account.match_index.to_le_bytes(),
        ],
        bump = match_account.bump,
        constraint = match_account.tournament == tournament.key()
            @ BracketChainError::InvalidMatchIndex,
    )]
    pub match_account: Box<Account<'info, MatchNode>>,

    #[account(
        seeds = [PARTICIPANT_SEED, tournament.key().as_ref(), match_account.player_a.as_ref()],
        bump = participant_a.bump,
    )]
    pub participant_a: Box<Account<'info, Participant>>,

    #[account(
        seeds = [PARTICIPANT_SEED, tournament.key().as_ref(), match_account.player_b.as_ref()],
        bump = participant_b.bump,
    )]
    pub participant_b: Box<Account<'info, Participant>>,
}

pub(crate) fn handler(ctx: Context<CommitMatchLobby>, lobby_id: [u8; 16]) -> Result<()> {
    require!(
        ctx.accounts.tournament.settlement_mode == SettlementMode::Oracle,
        BracketChainError::SettlementModeMismatch
    );
    require!(
        ctx.accounts.tournament.status == TournamentStatus::Active,
        BracketChainError::NotActive
    );
    require!(
        ctx.accounts.match_account.status == MatchStatus::Active,
        BracketChainError::MatchAlreadyReported
    );
    require!(
        ctx.accounts.match_account.commitment.is_none(),
        BracketChainError::MatchAlreadyCommitted
    );

    let clock = Clock::get()?;
    let now = clock.unix_timestamp;
    // `identity_hash` is the 32-byte SHA-256(steam_id_64 LE) fingerprint the
    // indexer issued in V1.1 (A-9); copied verbatim — no on-chain hashing here.
    let player_a_game_id = ctx.accounts.participant_a.identity_hash;
    let player_b_game_id = ctx.accounts.participant_b.identity_hash;

    let m = &mut ctx.accounts.match_account;
    m.commitment = Some(MatchCommitment {
        lobby_id,
        player_a_game_id,
        player_b_game_id,
        committed_at: now,
        committed_slot: clock.slot,
    });

    emit!(MatchLobbyCommitted {
        event_version: EVENT_VERSION_V1,
        tournament: m.tournament,
        bracket: m.bracket,
        round: m.round,
        match_index: m.match_index,
        lobby_id,
        committed_at: now,
    });

    Ok(())
}
