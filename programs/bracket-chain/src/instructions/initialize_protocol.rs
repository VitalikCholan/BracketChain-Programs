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

    /// CHECK: Treasury wallet — destination owner for protocol-fee USDC.
    /// The actual ATA `(treasury, usdc_mint)` is derived at distribution time.
    pub treasury: UncheckedAccount<'info>,

    pub usdc_mint: Account<'info, Mint>,

    pub system_program: Program<'info, System>,
}

pub(crate) fn handler(ctx: Context<InitializeProtocol>) -> Result<()> {
    let cfg = &mut ctx.accounts.protocol_config;
    cfg.authority = ctx.accounts.authority.key();
    cfg.treasury = ctx.accounts.treasury.key();
    cfg.usdc_mint = ctx.accounts.usdc_mint.key();
    cfg.fee_bps = PROTOCOL_FEE_BPS;
    cfg.bump = ctx.bumps.protocol_config;
    Ok(())
}
