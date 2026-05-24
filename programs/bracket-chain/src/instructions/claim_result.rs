use anchor_lang::prelude::*;
use anchor_spl::token::{Token, TokenAccount};

use crate::constants::{
    EVENT_VERSION_V1, MATCH_SEED, PARTICIPANT_SEED, TOURNAMENT_SEED, VAULT_SEED,
};
use crate::errors::BracketChainError;
use crate::events::ResultClaimed;
use crate::instructions::settlement::{credit_match_stats, finalize_match};
use crate::state::{MatchNode, Participant, ProposalSource, ProtocolConfig, Tournament};

/// Accounts for the two permissionless finalize paths — `claim_result`
/// (undisputed, after the dispute window) and `force_claim_disputed` (disputed,
/// after the 24h organizer-silence backstop). Both finalize the *proposed*
/// winner; the only difference is the precondition. The signer is any payer.
#[derive(Accounts)]
pub struct PermissionlessFinalize<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,

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
    pub match_account: Account<'info, MatchNode>,

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

    #[account(
        mut,
        constraint = organizer_token_account.mint == tournament.token_mint
            @ BracketChainError::InvalidTokenMint,
        constraint = organizer_token_account.owner == tournament.organizer
            @ BracketChainError::UnauthorizedAuthority,
    )]
    pub organizer_token_account: Option<Account<'info, TokenAccount>>,

    pub token_program: Program<'info, Token>,
}

/// Permissionlessly finalize an **undisputed** proposal once its dispute window
/// has elapsed. The trustless path that prevents a silent counterparty from
/// stalling the bracket.
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
    require!(!accs.match_account.disputed, BracketChainError::ProposalDisputed);
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
        &accs.organizer_token_account,
        &accs.protocol_config,
        &accs.token_program,
        ctx.remaining_accounts,
        winner,
        &placements,
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
        forced: false,
        claimed_at: now,
    });

    Ok(())
}
