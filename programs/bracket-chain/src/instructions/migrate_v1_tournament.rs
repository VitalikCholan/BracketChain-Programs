use anchor_lang::prelude::*;
use anchor_lang::system_program;
use anchor_lang::Discriminator;

use crate::constants::PROTOCOL_CONFIG_SEED;
use crate::errors::BracketChainError;
use crate::state::{ProtocolConfig, Tournament};

/// **Devnet upgrade-only.** Grows a pre-V1 `Tournament` account to the current
/// `Tournament::INIT_SPACE` so its appended V1.1 fields (game, settlement_mode,
/// dispute_window, VRF, …) decode as zero/defaults under the redeployed program.
/// Unnecessary for a fresh deploy.
///
/// Scope: only **Registration**-phase tournaments can be carried forward. The
/// C9 bracket-seed change (`bracket: u8` now in the MatchNode PDA seed) gives
/// already-initialized matches different addresses under V1, so an in-progress
/// bracket cannot be migrated — such tournaments must be cancelled & refunded.
#[derive(Accounts)]
pub struct MigrateV1Tournament<'info> {
    #[account(
        mut,
        address = protocol_config.authority @ BracketChainError::UnauthorizedAuthority,
    )]
    pub authority: Signer<'info>,

    #[account(
        seeds = [PROTOCOL_CONFIG_SEED],
        bump = protocol_config.bump,
    )]
    pub protocol_config: Account<'info, ProtocolConfig>,

    /// CHECK: not deserialized as `Account<Tournament>` — an un-migrated account
    /// is shorter than the current `INIT_SPACE` and would fail Anchor's
    /// deserialization. Validated below by program ownership + discriminator.
    #[account(mut)]
    pub tournament: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}

pub(crate) fn handler(ctx: Context<MigrateV1Tournament>) -> Result<()> {
    let ai = ctx.accounts.tournament.to_account_info();

    require_keys_eq!(*ai.owner, crate::ID, BracketChainError::InvalidTournamentAccount);

    {
        let data = ai.try_borrow_data()?;
        require!(data.len() >= 8, BracketChainError::InvalidTournamentAccount);
        require!(
            &data[..8] == Tournament::DISCRIMINATOR,
            BracketChainError::InvalidTournamentAccount
        );
    }

    let new_size = 8 + Tournament::INIT_SPACE;
    let cur_size = ai.data_len();
    // Idempotent: already at (or beyond) the V1 layout → nothing to do.
    require!(cur_size < new_size, BracketChainError::MigrationNotNeeded);

    // Top up rent for the larger account, then grow with zero-filled tail.
    let rent = Rent::get()?;
    let new_minimum = rent.minimum_balance(new_size);
    let delta = new_minimum.saturating_sub(ai.lamports());
    if delta > 0 {
        system_program::transfer(
            CpiContext::new(
                ctx.accounts.system_program.to_account_info(),
                system_program::Transfer {
                    from: ctx.accounts.authority.to_account_info(),
                    to: ai.clone(),
                },
            ),
            delta,
        )?;
    }

    ai.realloc(new_size, true)?;

    Ok(())
}
