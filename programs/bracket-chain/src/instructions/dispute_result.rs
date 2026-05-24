use anchor_lang::prelude::*;

use crate::constants::{EVENT_VERSION_V1, FORCE_CLAIM_WINDOW_SECS, MATCH_SEED, TOURNAMENT_SEED};
use crate::errors::BracketChainError;
use crate::events::ResultDisputed;
use crate::state::{MatchNode, ProposalSource, Tournament};

/// The counterparty rejects a pending proposal, routing the match to the
/// organizer arbitrator (`resolve_dispute`). Re-arms `claim_deadline` to
/// `now + FORCE_CLAIM_WINDOW_SECS` so that if the organizer stays silent, anyone
/// may `force_claim_disputed` after 24h (trustless backstop).
#[derive(Accounts)]
pub struct DisputeResult<'info> {
    pub disputer: Signer<'info>,

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

pub(crate) fn handler(ctx: Context<DisputeResult>, dispute_reason: u8) -> Result<()> {
    let disputer = ctx.accounts.disputer.key();
    let m = &mut ctx.accounts.match_account;

    require!(
        m.proposal_source != ProposalSource::None,
        BracketChainError::NoProposal
    );
    require!(!m.disputed, BracketChainError::ProposalDisputed);
    require!(
        disputer == m.player_a || disputer == m.player_b,
        BracketChainError::NotPlayerInMatch
    );
    // Only the side that did *not* author the proposal may dispute it. (For an
    // Oracle proposal the proposer is not a player, so either player qualifies.)
    require!(disputer != m.proposer, BracketChainError::NotCounterparty);

    let now = Clock::get()?.unix_timestamp;
    let force_claim_deadline = now
        .checked_add(FORCE_CLAIM_WINDOW_SECS)
        .ok_or(BracketChainError::ArithmeticOverflow)?;

    m.disputed = true;
    m.dispute_reason = dispute_reason;
    m.claim_deadline = force_claim_deadline;

    emit!(ResultDisputed {
        event_version: EVENT_VERSION_V1,
        tournament: m.tournament,
        bracket: m.bracket,
        round: m.round,
        match_index: m.match_index,
        disputer,
        dispute_reason,
        force_claim_deadline,
        disputed_at: now,
    });

    Ok(())
}
