use anchor_lang::prelude::*;
use switchboard_on_demand::accounts::RandomnessAccountData;

use crate::constants::{
    SWITCHBOARD_ON_DEMAND_DEVNET, SWITCHBOARD_ON_DEMAND_MAINNET, TOURNAMENT_SEED,
};
use crate::errors::BracketChainError;
use crate::state::Tournament;

/// Permissionless: read the revealed Switchboard randomness and lock it in as
/// the tournament's bracket seed. Switchboard On-Demand only returns the value
/// in the *same slot* it is revealed (`clock.slot == reveal_slot`), so the
/// off-chain caller (B-16 `vrf-reveal.cron`) must bundle Switchboard's reveal
/// instruction and this one in the same transaction.
#[derive(Accounts)]
pub struct RevealSeed<'info> {
    /// Permissionless fee-payer — anyone may push the reveal through.
    pub payer: Signer<'info>,

    #[account(
        mut,
        seeds = [
            TOURNAMENT_SEED,
            tournament.organizer.as_ref(),
            tournament.name.as_bytes(),
        ],
        bump = tournament.bump,
    )]
    pub tournament: Box<Account<'info, Tournament>>,

    /// CHECK: must be the exact account bound by `request_seed`; parsed as
    /// `RandomnessAccountData` below.
    #[account(address = tournament.vrf_randomness_account @ BracketChainError::RandomnessAccountMismatch)]
    pub randomness_account: UncheckedAccount<'info>,
}

pub(crate) fn handler(ctx: Context<RevealSeed>) -> Result<()> {
    require!(
        !ctx.accounts.tournament.seed_revealed,
        BracketChainError::SeedAlreadyRevealed
    );

    let owner = ctx.accounts.randomness_account.owner;
    require!(
        *owner == SWITCHBOARD_ON_DEMAND_DEVNET || *owner == SWITCHBOARD_ON_DEMAND_MAINNET,
        BracketChainError::InvalidRandomnessOwner
    );

    let clock = Clock::get()?;
    let value = {
        let data = ctx.accounts.randomness_account.data.borrow();
        let randomness = RandomnessAccountData::parse(data)
            .map_err(|_| error!(BracketChainError::MalformedRandomness))?;
        randomness
            .get_value(clock.slot)
            .map_err(|_| error!(BracketChainError::RandomnessNotResolved))?
    };

    let t = &mut ctx.accounts.tournament;
    t.seed_hash = value;
    t.seed_revealed = true;

    Ok(())
}
