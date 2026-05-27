use anchor_lang::prelude::*;

use crate::constants::PROTOCOL_CONFIG_SEED;
use crate::errors::BracketChainError;
use crate::state::ProtocolConfig;

/// Admin instruction (Stage C / V1.2): write the Switchboard On-Demand
/// settlement parameters onto `ProtocolConfig`. `propose_result_oracle` reads
/// `max_stale_slots` / `min_oracle_samples` when validating a feed, and
/// `bind_match_feed` checks bound feeds belong to `switchboard_queue`. The
/// On-Demand program id is the `SWITCHBOARD_ON_DEMAND_*` constant, not stored
/// here. Authority-gated; idempotent (callable again to retune). Mirrors
/// `set_sas_config`.
#[derive(Accounts)]
pub struct SetOracleConfig<'info> {
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
    ctx: Context<SetOracleConfig>,
    switchboard_queue: Pubkey,
    max_stale_slots: u32,
    min_oracle_samples: u32,
) -> Result<()> {
    let cfg = &mut ctx.accounts.protocol_config;
    cfg.switchboard_queue = switchboard_queue;
    cfg.max_stale_slots = max_stale_slots;
    cfg.min_oracle_samples = min_oracle_samples;
    Ok(())
}
