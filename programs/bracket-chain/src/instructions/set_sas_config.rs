use anchor_lang::prelude::*;

use crate::constants::PROTOCOL_CONFIG_SEED;
use crate::errors::BracketChainError;
use crate::state::ProtocolConfig;

/// Admin instruction: write BracketChain's SAS Credential + per-game Schema PDAs
/// onto `ProtocolConfig`. `join_tournament` validates incoming attestations
/// against these. Authority-gated; idempotent (callable again to rotate
/// credential or fill in a newly-activated game's schema slot).
#[derive(Accounts)]
pub struct SetSasConfig<'info> {
    pub authority: Signer<'info>,

    #[account(
        mut,
        seeds = [PROTOCOL_CONFIG_SEED],
        bump = protocol_config.bump,
        has_one = authority @ BracketChainError::UnauthorizedAuthority,
    )]
    pub protocol_config: Account<'info, ProtocolConfig>,
}

pub(crate) fn handler(
    ctx: Context<SetSasConfig>,
    sas_credential: Pubkey,
    sas_schemas: [Pubkey; 5],
) -> Result<()> {
    let cfg = &mut ctx.accounts.protocol_config;
    cfg.sas_credential = sas_credential;
    cfg.sas_schemas = sas_schemas;
    Ok(())
}
