use anchor_lang::prelude::*;

use crate::constants::{EVENT_VERSION_V1, TOURNAMENT_SEED};
use crate::errors::BracketChainError;
use crate::events::TournamentPartiallyCancelled;
use crate::state::{Tournament, TournamentStatus};

/// `partial_cancel_tournament` — organizer-signed mid-tournament cancellation
/// (Stage E, E-2). Callable only from `Active`; flips status to
/// `PartialCancelled` and freezes the bracket. Refunds are then processed
/// permissionlessly by `partial_refund_chunk` (Policy A: every participant is
/// refunded their full entry fee; the organizer recovers only their deposit —
/// cancelling is a pure loss, never a profit).
#[derive(Accounts)]
pub struct PartialCancelTournament<'info> {
    pub organizer: Signer<'info>,

    #[account(
        mut,
        seeds = [
            TOURNAMENT_SEED,
            tournament.organizer.as_ref(),
            tournament.name.as_bytes(),
        ],
        bump = tournament.bump,
        has_one = organizer @ BracketChainError::UnauthorizedAuthority,
    )]
    pub tournament: Account<'info, Tournament>,
}

pub(crate) fn handler(ctx: Context<PartialCancelTournament>) -> Result<()> {
    let tournament = &mut ctx.accounts.tournament;

    require!(
        tournament.status == TournamentStatus::Active,
        BracketChainError::TournamentInProgress
    );

    tournament.status = TournamentStatus::PartialCancelled;
    let now = Clock::get()?.unix_timestamp;

    emit!(TournamentPartiallyCancelled {
        event_version: EVENT_VERSION_V1,
        tournament: tournament.key(),
        authority: ctx.accounts.organizer.key(),
        cancelled_at: now,
    });

    Ok(())
}
