use anchor_lang::prelude::*;
use anchor_spl::token::{Token, TokenAccount};

use crate::constants::{MATCH_SEED, PARTICIPANT_SEED, TOURNAMENT_SEED, VAULT_SEED};
use crate::errors::BracketChainError;
use crate::instructions::settlement::{credit_match_stats, finalize_match};
use crate::state::{MatchNode, Participant, ProposalSource, ProtocolConfig, Tournament};

/// The counterparty accepts the pending proposal, finalizing the match
/// immediately for `proposed_winner`. Settles prizes if this is the final match
/// (same account contract as `report_result` — `placements` + ATAs in
/// `remaining_accounts`); pass empty / no ATAs for non-final matches.
#[derive(Accounts)]
pub struct ConfirmResult<'info> {
    pub counterparty: Signer<'info>,

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
    // Boxed: V1.2 grew MatchNode, pushing `try_accounts` over the SBF 4KB
    // stack frame. Deref-coercion keeps the finalize_match call compatible.
    pub match_account: Box<Account<'info, MatchNode>>,

    /// Required for non-final matches; pass `None` when finalizing the final.
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
    mut ctx: Context<'_, '_, '_, 'info, ConfirmResult<'info>>,
    placements: Vec<Pubkey>,
) -> Result<()> {
    let signer = ctx.accounts.counterparty.key();
    let accs = &mut ctx.accounts;

    require!(
        accs.match_account.proposal_source != ProposalSource::None,
        BracketChainError::NoProposal
    );
    require!(!accs.match_account.disputed, BracketChainError::ProposalDisputed);
    require!(
        signer == accs.match_account.player_a || signer == accs.match_account.player_b,
        BracketChainError::NotPlayerInMatch
    );
    require!(signer != accs.match_account.proposer, BracketChainError::NotCounterparty);

    let winner = accs.match_account.proposed_winner;
    let now = Clock::get()?.unix_timestamp;

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
        false, // counterparty-signed — WinnerTakesAll finals only (H-1)
        now,
    )?;

    credit_match_stats(&mut accs.participant_a, &mut accs.participant_b, winner)?;

    Ok(())
}
