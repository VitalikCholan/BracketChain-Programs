use anchor_lang::prelude::*;

use crate::constants::{EVENT_VERSION_V1, MATCH_SEED, TOURNAMENT_SEED};
use crate::errors::BracketChainError;
use crate::events::ResultProposed;
use crate::state::{MatchNode, MatchStatus, ProposalSource, SettlementMode, Tournament};

/// A match player records the result they claim. Opens the dispute window: the
/// counterparty may `confirm_result` (finalize now), `dispute_result` (escalate
/// to the organizer), or do nothing — after `dispute_window_secs`, anyone may
/// `claim_result` to finalize the unchallenged proposal.
#[derive(Accounts)]
pub struct ProposeResult<'info> {
    pub proposer: Signer<'info>,

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
    pub match_account: Account<'info, MatchNode>,
}

pub(crate) fn handler(ctx: Context<ProposeResult>, proposed_winner: Pubkey) -> Result<()> {
    let proposer = ctx.accounts.proposer.key();
    let m = &mut ctx.accounts.match_account;

    require!(
        ctx.accounts.tournament.settlement_mode != SettlementMode::OrganizerOnly,
        BracketChainError::SettlementModeMismatch
    );
    require!(m.status == MatchStatus::Active, BracketChainError::MatchAlreadyReported);
    require!(
        m.proposal_source == ProposalSource::None,
        BracketChainError::ProposalAlreadyExists
    );
    require!(
        proposer == m.player_a || proposer == m.player_b,
        BracketChainError::NotPlayerInMatch
    );
    require!(
        proposed_winner == m.player_a || proposed_winner == m.player_b,
        BracketChainError::InvalidProposedWinner
    );

    let now = Clock::get()?.unix_timestamp;
    let claim_deadline = now
        .checked_add(ctx.accounts.tournament.dispute_window_secs as i64)
        .ok_or(BracketChainError::ArithmeticOverflow)?;

    m.proposal_source = ProposalSource::Player;
    m.proposer = proposer;
    m.proposed_winner = proposed_winner;
    m.proposed_at = now;
    m.claim_deadline = claim_deadline;
    m.disputed = false;
    m.dispute_reason = 0;

    emit!(ResultProposed {
        event_version: EVENT_VERSION_V1,
        tournament: m.tournament,
        bracket: m.bracket,
        round: m.round,
        match_index: m.match_index,
        source: ProposalSource::Player as u8,
        proposer,
        proposed_winner,
        claim_deadline,
        proposed_at: now,
    });

    Ok(())
}
