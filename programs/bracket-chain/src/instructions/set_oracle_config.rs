use anchor_lang::prelude::*;

use crate::constants::{
    MAX_ORACLE_STALE_SLOTS_CEILING, MIN_ORACLE_SAMPLES_FLOOR, PROTOCOL_CONFIG_SEED,
};
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
    // L-2: reject misconfigurations that weaken oracle settlement. `get_value`'s
    // `submissions.len() < min_samples` guard is a no-op at 0 (would settle on a
    // single sample), and an unbounded staleness window would accept arbitrarily
    // old oracle data. See `validate_oracle_bounds`.
    validate_oracle_bounds(max_stale_slots, min_oracle_samples)?;

    let cfg = &mut ctx.accounts.protocol_config;
    cfg.switchboard_queue = switchboard_queue;
    cfg.max_stale_slots = max_stale_slots;
    cfg.min_oracle_samples = min_oracle_samples;
    Ok(())
}

/// Pure bounds check, factored out for unit testing.
fn validate_oracle_bounds(max_stale_slots: u32, min_oracle_samples: u32) -> Result<()> {
    require!(
        min_oracle_samples >= MIN_ORACLE_SAMPLES_FLOOR,
        BracketChainError::InvalidOracleConfig
    );
    require!(
        max_stale_slots <= MAX_ORACLE_STALE_SLOTS_CEILING,
        BracketChainError::InvalidOracleConfig
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_zero_samples() {
        // min_oracle_samples = 0 would let get_value settle on a single sample.
        assert!(validate_oracle_bounds(100, 0).is_err());
    }

    #[test]
    fn rejects_staleness_above_ceiling() {
        assert!(validate_oracle_bounds(MAX_ORACLE_STALE_SLOTS_CEILING + 1, 1).is_err());
    }

    #[test]
    fn accepts_bounds_edges() {
        // Floor sample count + max allowed staleness is the tightest valid config.
        assert!(validate_oracle_bounds(MAX_ORACLE_STALE_SLOTS_CEILING, MIN_ORACLE_SAMPLES_FLOOR).is_ok());
        // A typical retune well inside the window.
        assert!(validate_oracle_bounds(150, 3).is_ok());
    }
}
