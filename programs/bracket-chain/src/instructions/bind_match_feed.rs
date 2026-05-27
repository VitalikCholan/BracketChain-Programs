use anchor_lang::prelude::*;
use switchboard_on_demand::accounts::PullFeedAccountData;

use crate::constants::{
    EVENT_VERSION_V1, MATCH_SEED, PROTOCOL_CONFIG_SEED, SWITCHBOARD_ON_DEMAND_DEVNET,
    SWITCHBOARD_ON_DEMAND_MAINNET, TOURNAMENT_SEED,
};
use crate::errors::BracketChainError;
use crate::events::MatchFeedBound;
use crate::state::{MatchNode, ProtocolConfig, Tournament};

/// Stage C / V1.2: the organizer binds the Switchboard On-Demand `PullFeed`
/// (created off-chain by the indexer feed-factory) to a committed match. Split
/// from `commit_match_lobby` because feed creation is a multi-step TS flow that
/// happens after the lobby launches; commit is cheap and happens before.
///
/// The feed pubkey is the `switchboard_feed` account's own key. Validation:
/// owned by the On-Demand program, and on the protocol's configured queue.
#[derive(Accounts)]
pub struct BindMatchFeed<'info> {
    #[account(address = tournament.organizer @ BracketChainError::UnauthorizedAuthority)]
    pub organizer: Signer<'info>,

    #[account(
        seeds = [
            TOURNAMENT_SEED,
            tournament.organizer.as_ref(),
            tournament.name.as_bytes(),
        ],
        bump = tournament.bump,
    )]
    pub tournament: Box<Account<'info, Tournament>>,

    #[account(
        mut,
        seeds = [
            MATCH_SEED,
            tournament.key().as_ref(),
            &[match_account.bracket],
            &[match_account.round],
            &match_account.match_index.to_le_bytes(),
        ],
        bump = match_account.bump,
        constraint = match_account.tournament == tournament.key()
            @ BracketChainError::InvalidMatchIndex,
    )]
    pub match_account: Box<Account<'info, MatchNode>>,

    #[account(
        seeds = [PROTOCOL_CONFIG_SEED],
        bump = protocol_config.bump,
    )]
    pub protocol_config: Box<Account<'info, ProtocolConfig>>,

    /// CHECK: validated below — owner must be the On-Demand program and the
    /// parsed `PullFeedAccountData.queue` must match `protocol_config`.
    pub switchboard_feed: UncheckedAccount<'info>,
}

pub(crate) fn handler(ctx: Context<BindMatchFeed>) -> Result<()> {
    let commitment = ctx
        .accounts
        .match_account
        .commitment
        .ok_or(error!(BracketChainError::MatchNotCommitted))?;

    // 1. Owner must be the Switchboard On-Demand program.
    let owner = ctx.accounts.switchboard_feed.owner;
    require!(
        *owner == SWITCHBOARD_ON_DEMAND_DEVNET || *owner == SWITCHBOARD_ON_DEMAND_MAINNET,
        BracketChainError::WrongFeedAccount
    );

    // 2. Feed must live on the protocol's configured queue (set via
    //    `set_oracle_config`), and — the anti-redirection check (Layer 1) — its
    //    `feed_hash` must equal the `expected_feed_hash` committed for this
    //    match. The job schema bakes in `lobby_id` + both `player_*_game_id`, so
    //    this cryptographically binds the feed to *these* identities/lobby. The
    //    winner then arrives as a 0/1 index; identity is verified here, not in
    //    the (range-limited) feed value. Parsing also confirms it's a real feed.
    {
        let data = ctx.accounts.switchboard_feed.data.borrow();
        let feed = PullFeedAccountData::parse(data)
            .map_err(|_| error!(BracketChainError::WrongFeedAccount))?;
        require_keys_eq!(
            feed.queue,
            ctx.accounts.protocol_config.switchboard_queue,
            BracketChainError::WrongFeedAccount
        );
        require!(
            feed.feed_hash == commitment.expected_feed_hash,
            BracketChainError::WrongFeedAccount
        );
    }

    let feed_key = ctx.accounts.switchboard_feed.key();
    let m = &mut ctx.accounts.match_account;
    m.switchboard_feed = feed_key;

    emit!(MatchFeedBound {
        event_version: EVENT_VERSION_V1,
        tournament: m.tournament,
        bracket: m.bracket,
        round: m.round,
        match_index: m.match_index,
        switchboard_feed: feed_key,
    });

    Ok(())
}
