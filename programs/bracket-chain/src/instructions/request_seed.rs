use anchor_lang::prelude::*;
use switchboard_on_demand::accounts::RandomnessAccountData;

use crate::constants::{
    SWITCHBOARD_ON_DEMAND_DEVNET, SWITCHBOARD_ON_DEMAND_MAINNET, TOURNAMENT_SEED,
};
use crate::errors::BracketChainError;
use crate::state::{SettlementMode, Tournament, TournamentStatus};

/// Binds a Switchboard On-Demand randomness account to the tournament for
/// verifiable bracket seeding. The organizer creates + commits the randomness
/// account client-side (Switchboard SDK `commitIx`); this instruction records
/// the binding so `reveal_seed` can only consume *this* account, and gates
/// `start_tournament` behind the reveal.
///
/// Opt-in: a tournament that never calls `request_seed` keeps the slot-hash
/// seed (see `start_tournament`). Once bound, `start` requires `seed_revealed`.
#[derive(Accounts)]
pub struct RequestSeed<'info> {
    #[account(address = tournament.organizer @ BracketChainError::UnauthorizedAuthority)]
    pub organizer: Signer<'info>,

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

    /// CHECK: validated below — must be owned by the Switchboard On-Demand
    /// program and parse as a `RandomnessAccountData`.
    pub randomness_account: UncheckedAccount<'info>,
}

pub(crate) fn handler(ctx: Context<RequestSeed>) -> Result<()> {
    // Must bind before the bracket is drawn, and only where seeding fairness
    // matters (player-reported / oracle modes).
    require!(
        ctx.accounts.tournament.status == TournamentStatus::Registration,
        BracketChainError::NotInRegistration
    );
    require!(
        ctx.accounts.tournament.settlement_mode != SettlementMode::OrganizerOnly,
        BracketChainError::SettlementModeMismatch
    );
    require!(
        !ctx.accounts.tournament.seed_revealed,
        BracketChainError::SeedAlreadyRevealed
    );

    let owner = ctx.accounts.randomness_account.owner;
    require!(
        *owner == SWITCHBOARD_ON_DEMAND_DEVNET || *owner == SWITCHBOARD_ON_DEMAND_MAINNET,
        BracketChainError::InvalidRandomnessOwner
    );

    // Confirm it really is a randomness account (discriminator + layout).
    {
        let data = ctx.accounts.randomness_account.data.borrow();
        RandomnessAccountData::parse(data)
            .map_err(|_| error!(BracketChainError::MalformedRandomness))?;
    }

    let clock = Clock::get()?;
    let t = &mut ctx.accounts.tournament;
    t.vrf_randomness_account = ctx.accounts.randomness_account.key();
    t.vrf_commit_slot = clock.slot;

    Ok(())
}
