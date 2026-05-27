use anchor_lang::prelude::*;
use switchboard_on_demand::accounts::PullFeedAccountData;
use switchboard_on_demand::prelude::rust_decimal::prelude::ToPrimitive;

use crate::constants::{
    EVENT_VERSION_V1, MATCH_SEED, PROTOCOL_CONFIG_SEED, TOURNAMENT_SEED,
};
use crate::errors::BracketChainError;
use crate::events::ResultProposed;
use crate::state::{MatchNode, MatchStatus, ProposalSource, ProtocolConfig, SettlementMode, Tournament};

/// Stage C / V1.2 — **permissionless** Oracle proposal. Reads the Switchboard
/// On-Demand feed bound to a committed match, derives the winning Steam ID, and
/// writes it into Stage B's proposal envelope with `source = Oracle`. From here
/// the lifecycle is V1's unchanged: dispute window → `claim_result`, or
/// `dispute_result` (broadened signer) → `resolve_dispute` / `force_claim`.
///
/// **Feed value contract:** the `OracleJob` returns the winning **Steam ID 64**
/// as a plain integer (it fits a Switchboard `Decimal`/`i128`; a 256-bit
/// identity hash does not). The program hashes it on-chain —
/// `SHA-256(steam_id_64 as u64 LE)` — exactly as the indexer's SAS issuer did
/// in V1.1 (A-9), and matches against the committed `player_*_game_id`.
#[derive(Accounts)]
pub struct ProposeResultOracle<'info> {
    /// Permissionless relayer / fee-payer — anyone may push the proposal; trust
    /// bottoms out in the feed account contents, not the signer.
    pub relayer: Signer<'info>,

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

    /// CHECK: must equal `match_account.switchboard_feed`; parsed below.
    pub switchboard_feed: UncheckedAccount<'info>,
}

pub(crate) fn handler(ctx: Context<ProposeResultOracle>) -> Result<()> {
    require!(
        ctx.accounts.tournament.settlement_mode == SettlementMode::Oracle,
        BracketChainError::SettlementModeMismatch
    );
    require!(
        ctx.accounts.match_account.status == MatchStatus::Active,
        BracketChainError::MatchAlreadyReported
    );
    require!(
        ctx.accounts.match_account.proposal_source == ProposalSource::None,
        BracketChainError::ProposalAlreadyExists
    );
    let commitment = ctx
        .accounts
        .match_account
        .commitment
        .ok_or(error!(BracketChainError::MatchNotCommitted))?;

    // The feed must be the exact one bound by `bind_match_feed`.
    require_keys_eq!(
        ctx.accounts.switchboard_feed.key(),
        ctx.accounts.match_account.switchboard_feed,
        BracketChainError::WrongFeedAccount
    );

    // Read the feed value (median of fresh oracle samples) → winning Steam ID.
    let clock = Clock::get()?;
    let winner_steam_id: u64 = {
        let data = ctx.accounts.switchboard_feed.data.borrow();
        let feed = PullFeedAccountData::parse(data)
            .map_err(|_| error!(BracketChainError::WrongFeedAccount))?;
        let value = feed
            .get_value(
                clock.slot,
                ctx.accounts.protocol_config.max_stale_slots as u64,
                ctx.accounts.protocol_config.min_oracle_samples,
                true, // only_positive — Steam IDs are positive
            )
            .map_err(|_| error!(BracketChainError::OracleWinnerNotInMatch))?;
        // Truncate the scaled Decimal to its integer part. A Steam ID 64
        // (~7.6e16) fits u64 with room to spare.
        value
            .to_u64()
            .ok_or(error!(BracketChainError::OracleWinnerNotInMatch))?
    };

    // Hash on-chain exactly as A-9: SHA-256 over the 8-byte LE encoding.
    let winner_hash: [u8; 32] =
        solana_sha256_hasher::hash(&winner_steam_id.to_le_bytes()).to_bytes();

    let m = &mut ctx.accounts.match_account;
    let winner = if winner_hash == commitment.player_a_game_id {
        m.player_a
    } else if winner_hash == commitment.player_b_game_id {
        m.player_b
    } else {
        return err!(BracketChainError::OracleWinnerNotInMatch);
    };

    let now = clock.unix_timestamp;
    let claim_deadline = now
        .checked_add(ctx.accounts.tournament.dispute_window_secs as i64)
        .ok_or(BracketChainError::ArithmeticOverflow)?;
    let relayer = ctx.accounts.relayer.key();

    m.proposal_source = ProposalSource::Oracle;
    m.proposer = relayer;
    m.proposed_winner = winner;
    m.proposed_at = now;
    m.claim_deadline = claim_deadline;
    m.disputed = false;
    m.dispute_reason = 0;

    emit!(ResultProposed {
        event_version: EVENT_VERSION_V1,
        tournament: m.tournament,
        bracket: m.bracket,
        round: m.round,
        match_index: m.match_index,
        source: ProposalSource::Oracle as u8,
        proposer: relayer,
        proposed_winner: winner,
        claim_deadline,
        proposed_at: now,
    });

    Ok(())
}
