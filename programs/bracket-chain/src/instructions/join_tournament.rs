use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer};

use crate::constants::{PARTICIPANT_SEED, VAULT_SEED};
use crate::errors::BracketChainError;
use crate::events::ParticipantRegistered;
use crate::state::{Participant, Tournament, TournamentStatus};

#[derive(Accounts)]
pub struct JoinTournament<'info> {
    #[account(mut)]
    pub player: Signer<'info>,

    #[account(
        mut,
        seeds = [
            crate::constants::TOURNAMENT_SEED,
            tournament.organizer.as_ref(),
            tournament.name.as_bytes(),
        ],
        bump = tournament.bump,
    )]
    pub tournament: Account<'info, Tournament>,

    #[account(
        init,
        payer = player,
        space = 8 + Participant::INIT_SPACE,
        seeds = [PARTICIPANT_SEED, tournament.key().as_ref(), player.key().as_ref()],
        bump,
    )]
    pub participant: Account<'info, Participant>,

    #[account(
        mut,
        constraint = player_token_account.mint == tournament.usdc_mint
            @ BracketChainError::InvalidUsdcMint,
        constraint = player_token_account.owner == player.key()
            @ BracketChainError::UnauthorizedAuthority,
    )]
    pub player_token_account: Account<'info, TokenAccount>,

    #[account(
        mut,
        seeds = [VAULT_SEED, tournament.key().as_ref()],
        bump = tournament.vault_bump,
    )]
    pub vault: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

pub(crate) fn handler(ctx: Context<JoinTournament>) -> Result<()> {
    let tournament = &mut ctx.accounts.tournament;

    require!(
        tournament.status == TournamentStatus::Registration,
        BracketChainError::NotInRegistration
    );

    let now = Clock::get()?.unix_timestamp;
    require!(
        now < tournament.registration_deadline,
        BracketChainError::RegistrationClosed
    );

    require!(
        tournament.participant_count < tournament.max_participants,
        BracketChainError::TournamentFull
    );

    let participant_index = tournament.participant_count;

    let cpi_accounts = Transfer {
        from: ctx.accounts.player_token_account.to_account_info(),
        to: ctx.accounts.vault.to_account_info(),
        authority: ctx.accounts.player.to_account_info(),
    };
    let cpi_ctx = CpiContext::new(ctx.accounts.token_program.to_account_info(), cpi_accounts);
    token::transfer(cpi_ctx, tournament.entry_fee)?;

    let participant = &mut ctx.accounts.participant;
    participant.tournament = tournament.key();
    participant.wallet = ctx.accounts.player.key();
    participant.seed_index = participant_index;
    participant.refund_paid = false;
    participant.bump = ctx.bumps.participant;

    tournament.participant_count = tournament
        .participant_count
        .checked_add(1)
        .ok_or(BracketChainError::ArithmeticOverflow)?;

    emit!(ParticipantRegistered {
        tournament: tournament.key(),
        wallet: participant.wallet,
        participant_index,
    });

    Ok(())
}
