use anchor_lang::prelude::*;

use crate::constants::EVENT_VERSION_V1;
use crate::errors::BracketChainError;
use crate::events::ResultClaimed;
use crate::instructions::claim_result::PermissionlessFinalize;
use crate::instructions::settlement::{credit_match_stats, finalize_match};
use crate::state::ProposalSource;

/// Permissionless backstop against an organizer who never resolves a dispute.
/// Once `FORCE_CLAIM_WINDOW_SECS` (24h) has elapsed since the dispute — tracked
/// via the re-armed `claim_deadline` — anyone may finalize the originally
/// proposed winner. Reuses [`PermissionlessFinalize`] (see `claim_result`).
pub(crate) fn handler<'info>(
    mut ctx: Context<'_, '_, '_, 'info, PermissionlessFinalize<'info>>,
    placements: Vec<Pubkey>,
) -> Result<()> {
    let accs = &mut ctx.accounts;
    let now = Clock::get()?.unix_timestamp;

    require!(
        accs.match_account.proposal_source != ProposalSource::None,
        BracketChainError::NoProposal
    );
    require!(accs.match_account.disputed, BracketChainError::ProposalNotDisputed);
    require!(
        now >= accs.match_account.claim_deadline,
        BracketChainError::ClaimWindowNotElapsed
    );

    let winner = accs.match_account.proposed_winner;
    let (tournament_key, bracket, round, match_index) = (
        accs.match_account.tournament,
        accs.match_account.bracket,
        accs.match_account.round,
        accs.match_account.match_index,
    );

    finalize_match(
        &mut accs.tournament,
        &mut accs.match_account,
        &mut accs.next_match,
        &mut accs.vault,
        &accs.protocol_config,
        &accs.token_program,
        ctx.remaining_accounts,
        winner,
        &placements,
        false, // permissionless — WinnerTakesAll finals only (H-1)
        now,
    )?;

    credit_match_stats(&mut accs.participant_a, &mut accs.participant_b, winner)?;

    emit!(ResultClaimed {
        event_version: EVENT_VERSION_V1,
        tournament: tournament_key,
        bracket,
        round,
        match_index,
        winner,
        forced: true,
        claimed_at: now,
    });

    Ok(())
}
