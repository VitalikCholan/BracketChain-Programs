use anchor_lang::prelude::*;
use anchor_lang::system_program;
use anchor_lang::Discriminator;

use crate::constants::PROTOCOL_CONFIG_SEED;
use crate::errors::BracketChainError;
use crate::state::ProtocolConfig;

/// Admin one-shot: realloc `ProtocolConfig` to the current `INIT_SPACE` so
/// downstream V1.1 (SAS) + V1.2 (Oracle) fields decode under the upgraded
/// program. Mirrors the pattern in `migrate_v1_tournament` — the account is
/// loaded as `UncheckedAccount` because Anchor's `Account<T>` deserializer
/// runs *before* `realloc` and would reject the smaller pre-V1.1 layout.
///
/// Authority is validated manually by reading bytes 8..40 (the `authority`
/// field of the on-chain struct, always at the same offset in all versions).
///
/// Idempotent: once the account is at or beyond the current size we return
/// `MigrationNotNeeded` so a redundant call doesn't waste rent.
#[derive(Accounts)]
pub struct MigrateProtocolConfig<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    /// CHECK: not deserialized as `Account<ProtocolConfig>` — the pre-V1.1
    /// layout (107 bytes) is shorter than the current INIT_SPACE and would
    /// fail Anchor's deserialization. Program ownership + discriminator +
    /// authority are validated below.
    #[account(
        mut,
        seeds = [PROTOCOL_CONFIG_SEED],
        bump,
    )]
    pub protocol_config: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}

pub(crate) fn handler(ctx: Context<MigrateProtocolConfig>) -> Result<()> {
    let ai = ctx.accounts.protocol_config.to_account_info();

    require_keys_eq!(*ai.owner, crate::ID, BracketChainError::UnauthorizedAuthority);

    // Validate discriminator + authority field (offset 8..40 — Pubkey).
    {
        let data = ai.try_borrow_data()?;
        require!(data.len() >= 40, BracketChainError::UnauthorizedAuthority);
        require!(
            &data[..8] == ProtocolConfig::DISCRIMINATOR,
            BracketChainError::UnauthorizedAuthority
        );
        let stored_authority = Pubkey::try_from(&data[8..40])
            .map_err(|_| error!(BracketChainError::UnauthorizedAuthority))?;
        require_keys_eq!(
            stored_authority,
            ctx.accounts.authority.key(),
            BracketChainError::UnauthorizedAuthority
        );
    }

    let new_size = 8 + ProtocolConfig::INIT_SPACE;
    let cur_size = ai.data_len();
    require!(cur_size < new_size, BracketChainError::MigrationNotNeeded);

    // Top up rent for the larger account, then grow with zero-filled tail.
    // Zero bytes deserialize as `Pubkey::default()` / `0` — the correct
    // "unset" sentinels for the appended V1.1/V1.2 fields, which authority
    // then populates via `set_sas_config` / `set_oracle_config`.
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
