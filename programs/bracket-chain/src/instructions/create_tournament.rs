use anchor_lang::prelude::*;
use anchor_spl::token::{self, Mint, Token, TokenAccount, Transfer};

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

    /// SPL Token mint for the tournament's prize pool. Any valid SPL mint is
    /// accepted (USDC, wSOL for native-SOL tournaments via wrap, custom).
    /// Frontend gatekeeps user-facing token selection — on-chain trusts the
    /// caller. `Account<Mint>` validates the account is a real Mint.
    pub token_mint: Account<'info, Mint>,

    /// Tournament PDA. `name` is used directly as a seed; capped at
    /// `MAX_TOURNAMENT_NAME_LEN` (32) bytes — Solana's per-seed limit. Length
    /// validated in the handler before account init.
    #[account(
        init,
        payer = organizer,
        space = 8 + Tournament::INIT_SPACE,
        seeds = [
            TOURNAMENT_SEED,
            organizer.key().as_ref(),
            name.as_bytes(),
        ],
        bump,
    )]
    pub tournament: Account<'info, Tournament>,

    #[account(
        init,
        payer = organizer,
        seeds = [VAULT_SEED, tournament.key().as_ref()],
        bump,
        token::mint = token_mint,
        token::authority = tournament,
    )]
    pub vault: Account<'info, TokenAccount>,

    /// Optional organizer ATA used to fund `organizer_deposit`. Required when
    /// `organizer_deposit > 0`; pass `None` to skip. Mint + owner constraints
    /// guarantee the deposit is debited from the organizer's own funds in the
    /// configured tournament token. Anchor 0.32 auto-unwraps `Option<Account>`
    /// inside constraint expressions and skips the check when the account is
    /// `None`, so we reference fields directly without explicit Option handling.
    #[account(
        mut,
        constraint = organizer_token_account.mint == token_mint.key()
            @ BracketChainError::InvalidTokenMint,
        constraint = organizer_token_account.owner == organizer.key()
            @ BracketChainError::UnauthorizedAuthority,
    )]
    pub organizer_token_account: Option<Account<'info, TokenAccount>>,

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
    organizer_deposit: u64,
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

    if organizer_deposit > 0 {
        let organizer_ata = ctx
            .accounts
            .organizer_token_account
            .as_ref()
            .ok_or(error!(BracketChainError::InvalidVault))?;

        token::transfer(
            CpiContext::new(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: organizer_ata.to_account_info(),
                    to: ctx.accounts.vault.to_account_info(),
                    authority: ctx.accounts.organizer.to_account_info(),
                },
            ),
            organizer_deposit,
        )?;
    }

    let tournament = &mut ctx.accounts.tournament;
    tournament.organizer = ctx.accounts.organizer.key();
    tournament.name = name;
    tournament.token_mint = ctx.accounts.token_mint.key();
    tournament.vault = ctx.accounts.vault.key();
    tournament.entry_fee = entry_fee;
    tournament.organizer_deposit = organizer_deposit;
    tournament.organizer_deposit_refunded = false;
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
        token_mint: tournament.token_mint,
        entry_fee,
        organizer_deposit,
        max_participants,
        payout_preset: payout_preset_discriminator(payout_preset),
        registration_deadline,
        name: tournament.name.clone(),
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
