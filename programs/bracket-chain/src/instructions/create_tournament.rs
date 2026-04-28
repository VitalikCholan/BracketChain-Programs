use anchor_lang::prelude::*;
use anchor_spl::token::{Mint, Token, TokenAccount};

use crate::constants::{
    MAX_PARTICIPANTS, MAX_TOURNAMENT_NAME_LEN, MIN_PARTICIPANTS, TOURNAMENT_SEED, VAULT_SEED,
};
use crate::errors::BracketChainError;
use crate::events::TournamentCreated;
use crate::state::{PayoutPreset, ProtocolConfig, Tournament, TournamentStatus};

#[derive(Accounts)]
#[instruction(name: String)]
pub struct CreateTournament<'info> {
    #[account(mut)]
    pub organizer: Signer<'info>,

    #[account(
        seeds = [crate::constants::PROTOCOL_CONFIG_SEED],
        bump = protocol_config.bump,
    )]
    pub protocol_config: Account<'info, ProtocolConfig>,

    #[account(
        address = protocol_config.usdc_mint @ BracketChainError::InvalidUsdcMint,
    )]
    pub usdc_mint: Account<'info, Mint>,

    #[account(
        init,
        payer = organizer,
        space = 8 + Tournament::INIT_SPACE,
        seeds = [TOURNAMENT_SEED, organizer.key().as_ref(), name.as_bytes()],
        bump,
    )]
    pub tournament: Account<'info, Tournament>,

    #[account(
        init,
        payer = organizer,
        seeds = [VAULT_SEED, tournament.key().as_ref()],
        bump,
        token::mint = usdc_mint,
        token::authority = tournament,
    )]
    pub vault: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
    pub rent: Sysvar<'info, Rent>,
}

pub(crate) fn handler(
    ctx: Context<CreateTournament>,
    name: String,
    entry_fee: u64,
    max_participants: u16,
    payout_preset: PayoutPreset,
    registration_deadline: i64,
) -> Result<()> {
    require!(
        name.as_bytes().len() <= MAX_TOURNAMENT_NAME_LEN,
        BracketChainError::NameTooLong
    );
    require!(
        max_participants >= MIN_PARTICIPANTS,
        BracketChainError::MinParticipantsNotMet
    );
    require!(
        max_participants <= MAX_PARTICIPANTS,
        BracketChainError::MaxParticipantsExceeded
    );
    require!(
        payout_preset.min_participants() <= max_participants,
        BracketChainError::PresetExceedsParticipants
    );

    let now = Clock::get()?.unix_timestamp;
    require!(
        registration_deadline > now,
        BracketChainError::RegistrationClosed
    );

    let tournament = &mut ctx.accounts.tournament;
    tournament.organizer = ctx.accounts.organizer.key();
    tournament.name = name;
    tournament.usdc_mint = ctx.accounts.usdc_mint.key();
    tournament.vault = ctx.accounts.vault.key();
    tournament.entry_fee = entry_fee;
    tournament.max_participants = max_participants;
    tournament.bracket_size = 0;
    tournament.participant_count = 0;
    tournament.matches_initialized = 0;
    tournament.matches_reported = 0;
    tournament.total_matches = 0;
    tournament.registration_deadline = registration_deadline;
    tournament.created_at = now;
    tournament.started_at = 0;
    tournament.completed_at = 0;
    tournament.status = TournamentStatus::Registration;
    tournament.payout_preset = payout_preset;
    tournament.seed_hash = [0u8; 32];
    tournament.champion = Pubkey::default();
    tournament.bump = ctx.bumps.tournament;
    tournament.vault_bump = ctx.bumps.vault;

    emit!(TournamentCreated {
        tournament: tournament.key(),
        organizer: tournament.organizer,
        usdc_mint: tournament.usdc_mint,
        entry_fee,
        max_participants,
        payout_preset: payout_preset_discriminator(payout_preset),
        registration_deadline,
    });

    Ok(())
}

fn payout_preset_discriminator(preset: PayoutPreset) -> u8 {
    match preset {
        PayoutPreset::WinnerTakesAll => 0,
        PayoutPreset::Standard => 1,
        PayoutPreset::Deep => 2,
    }
}
