use anchor_lang::prelude::*;
use anchor_spl::token::Mint;

use crate::constants::{PROTOCOL_CONFIG_SEED, PROTOCOL_FEE_BPS};
use crate::state::ProtocolConfig;

#[derive(Accounts)]
pub struct InitializeProtocol<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(
        init,
        payer = authority,
        space = 8 + ProtocolConfig::INIT_SPACE,
        seeds = [PROTOCOL_CONFIG_SEED],
        bump,
    )]
    pub protocol_config: Account<'info, ProtocolConfig>,

    /// CHECK: Treasury wallet — destination owner for protocol-fee tokens.
    /// The actual ATA `(treasury, tournament.token_mint)` is derived at
    /// distribution time per tournament.
    pub treasury: UncheckedAccount<'info>,

    /// Recommended default mint (e.g. USDC). Stored on `ProtocolConfig` as
    /// advisory metadata — per-tournament `token_mint` is not constrained
    /// against this.
    pub default_mint: Account<'info, Mint>,

    pub system_program: Program<'info, System>,
}

pub(crate) fn handler(ctx: Context<InitializeProtocol>) -> Result<()> {
    let cfg = &mut ctx.accounts.protocol_config;
    cfg.authority = ctx.accounts.authority.key();
    cfg.treasury = ctx.accounts.treasury.key();
    cfg.default_mint = ctx.accounts.default_mint.key();
    cfg.fee_bps = PROTOCOL_FEE_BPS;
    cfg.bump = ctx.bumps.protocol_config;
    Ok(())
}
