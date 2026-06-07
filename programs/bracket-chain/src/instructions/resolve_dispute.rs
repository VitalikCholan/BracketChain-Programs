use anchor_lang::prelude::*;
use anchor_spl::token::{Token, TokenAccount};

use crate::constants::{
    EVENT_VERSION_V1, MATCH_SEED, PARTICIPANT_SEED, TOURNAMENT_SEED, VAULT_SEED,
};
use crate::errors::BracketChainError;
use crate::events::DisputeResolved;
use crate::instructions::settlement::{credit_match_stats, finalize_match};
use crate::state::{MatchNode, Participant, ProtocolConfig, Tournament};

/// The organizer (arbitrator) settles a disputed match, choosing the winner.
/// May be called any time after a dispute is raised — it is the organizer's
/// counterpart to the players' `force_claim_disputed` backstop.
#[derive(Accounts)]
pub struct ResolveDispute<'info> {
    #[account(address = tournament.organizer @ BracketChainError::UnauthorizedAuthority)]
    pub organizer: Signer<'info>,

    #[account(
        mut,
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
    // Boxed: V1.2 grew MatchNode (+`commitment`/`switchboard_feed`), pushing
    // this struct's `try_accounts` over the SBF 4KB stack frame. Heap-allocate
    // the larger of the two MatchNodes; deref-coercion keeps the finalize_match
    // call compatible. (`next_match` stays unboxed — `&mut Option<Box<_>>`
    // would not coerce to the `&mut Option<Account>` parameter.)
    pub match_account: Box<Account<'info, MatchNode>>,

    #[account(mut)]
    pub next_match: Option<Account<'info, MatchNode>>,

    #[account(mut, seeds = [PARTICIPANT_SEED, tournament.key().as_ref(), match_account.player_a.as_ref()], bump = participant_a.bump)]
    pub participant_a: Box<Account<'info, Participant>>,

    #[account(mut, seeds = [PARTICIPANT_SEED, tournament.key().as_ref(), match_account.player_b.as_ref()], bump = participant_b.bump)]
    pub participant_b: Box<Account<'info, Participant>>,

    #[account(
        seeds = [crate::constants::PROTOCOL_CONFIG_SEED],
        bump = protocol_config.bump,
    )]
    pub protocol_config: Box<Account<'info, ProtocolConfig>>,

    #[account(
        mut,
        seeds = [VAULT_SEED, tournament.key().as_ref()],
        bump = tournament.vault_bump,
        constraint = vault.key() == tournament.vault @ BracketChainError::InvalidVault,
    )]
    pub vault: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
}

pub(crate) fn handler<'info>(
    mut ctx: Context<'_, '_, '_, 'info, ResolveDispute<'info>>,
    winner: Pubkey,
    placements: Vec<Pubkey>,
) -> Result<()> {
    let arbitrator = ctx.accounts.organizer.key();
    let accs = &mut ctx.accounts;

    require!(accs.match_account.disputed, BracketChainError::ProposalNotDisputed);

    let (tournament_key, bracket, round, match_index) = (
        accs.match_account.tournament,
        accs.match_account.bracket,
        accs.match_account.round,
        accs.match_account.match_index,
    );
    let now = Clock::get()?.unix_timestamp;

    // `finalize_match` enforces winner ∈ {player_a, player_b}.
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
        true, // organizer/arbitrator-signed — trusted to adjudicate placements (H-1)
        now,
    )?;

    credit_match_stats(&mut accs.participant_a, &mut accs.participant_b, winner)?;

    emit!(DisputeResolved {
        event_version: EVENT_VERSION_V1,
        tournament: tournament_key,
        bracket,
        round,
        match_index,
        arbitrator,
        winner,
        resolved_at: now,
    });

    Ok(())
}
