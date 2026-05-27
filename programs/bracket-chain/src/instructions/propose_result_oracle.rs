use anchor_lang::prelude::*;
use switchboard_on_demand::accounts::PullFeedAccountData;
use switchboard_on_demand::prelude::rust_decimal::prelude::ToPrimitive;
use switchboard_on_demand::prelude::rust_decimal::Decimal;

use crate::constants::{
    EVENT_VERSION_V1, MATCH_SEED, PROTOCOL_CONFIG_SEED, TOURNAMENT_SEED,
};
use crate::errors::BracketChainError;
use crate::events::ResultProposed;
use crate::state::{
    MatchNode, MatchStatus, ProposalSource, ProtocolConfig, SettlementMode, Tournament,
};

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
    // Must be committed (identity binding lives in `bind_match_feed` via the
    // feed_hash check — Layer 1).
    require!(
        ctx.accounts.match_account.commitment.is_some(),
        BracketChainError::MatchNotCommitted
    );

    // The feed must be the exact one bound by `bind_match_feed`.
    require_keys_eq!(
        ctx.accounts.switchboard_feed.key(),
        ctx.accounts.match_account.switchboard_feed,
        BracketChainError::WrongFeedAccount
    );

    // Read the feed value (median of fresh oracle samples) → winner index.
    let clock = Clock::get()?;
    let value = {
        let data = ctx.accounts.switchboard_feed.data.borrow();
        let feed = PullFeedAccountData::parse(data)
            .map_err(|_| error!(BracketChainError::WrongFeedAccount))?;
        // only_positive = false: the winner index legitimately includes 0
        // (= player_a); `resolve_oracle_winner` whitelists exactly {0, 1}.
        feed.get_value(
            clock.slot,
            ctx.accounts.protocol_config.max_stale_slots as u64,
            ctx.accounts.protocol_config.min_oracle_samples,
            false,
        )
        .map_err(|_| error!(BracketChainError::OracleWinnerNotInMatch))?
    };

    let m = &mut ctx.accounts.match_account;
    let winner = resolve_oracle_winner(value, m.player_a, m.player_b)?;

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

/// Pure winner resolution, factored out of `handler` for unit testing (C-8).
///
/// The feed reports the **winner index** — `0` = player_a, `1` = player_b — not
/// an identity hash: a Switchboard `Decimal` (96-bit mantissa, max ~7.9e28)
/// cannot carry a Steam ID (7.6e16) scaled by 10^18, let alone a 256-bit hash.
/// Identity is instead pinned by `bind_match_feed`'s `feed_hash` check, so a
/// 0/1 index off a correctly-bound feed unambiguously names a committed player.
/// Anything other than {0, 1} (incl. a negative or fractional median from a
/// misbehaving feed) is rejected.
fn resolve_oracle_winner(value: Decimal, player_a: Pubkey, player_b: Pubkey) -> Result<Pubkey> {
    match value.to_u64() {
        Some(0) => Ok(player_a),
        Some(1) => Ok(player_b),
        _ => err!(BracketChainError::OracleWinnerNotInMatch),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytemuck::Zeroable;
    use switchboard_on_demand::accounts::OracleSubmission;

    /// Switchboard scales the submitted integer by 10^PRECISION (18).
    fn scaled(index: u64) -> i128 {
        (index as i128) * 10i128.pow(18)
    }

    /// A feed reporting `value` from `n` fresh oracle samples at `slot`.
    fn feed_reporting(value: i128, n: usize, slot: u64) -> PullFeedAccountData {
        let mut feed = PullFeedAccountData::zeroed();
        for i in 0..n {
            feed.submissions[i] = OracleSubmission {
                oracle: Pubkey::default(),
                slot,
                landed_at: slot,
                value,
            };
        }
        feed
    }

    #[test]
    fn index_0_resolves_player_a() {
        let (a, b) = (Pubkey::new_unique(), Pubkey::new_unique());
        assert_eq!(resolve_oracle_winner(Decimal::from(0u64), a, b).unwrap(), a);
    }

    #[test]
    fn index_1_resolves_player_b() {
        let (a, b) = (Pubkey::new_unique(), Pubkey::new_unique());
        assert_eq!(resolve_oracle_winner(Decimal::from(1u64), a, b).unwrap(), b);
    }

    #[test]
    fn out_of_range_index_is_rejected() {
        let (a, b) = (Pubkey::new_unique(), Pubkey::new_unique());
        assert!(resolve_oracle_winner(Decimal::from(2u64), a, b).is_err());
        // A negative median (misbehaving feed) → to_u64 None → rejected.
        assert!(resolve_oracle_winner(Decimal::from(-1i64), a, b).is_err());
    }

    #[test]
    fn end_to_end_pullfeed_get_value_to_winner() {
        // Build a real PullFeedAccountData reporting index 1 (player_b), run
        // Switchboard's own get_value (only_positive=false, as the handler
        // does), then resolve — validates the scaling + extraction chain.
        let feed = feed_reporting(scaled(1), 5, 100);
        let value = feed
            .get_value(100, 100, 5, false)
            .expect("fresh + enough samples");
        assert_eq!(value.to_u64().unwrap(), 1);

        let (a, b) = (Pubkey::new_unique(), Pubkey::new_unique());
        assert_eq!(resolve_oracle_winner(value, a, b).unwrap(), b);
    }

    #[test]
    fn index_0_survives_get_value_with_only_positive_false() {
        // The reason the handler passes only_positive=false: index 0 (player_a)
        // is a legitimate winner the `true` variant would reject as `<= 0`.
        let feed = feed_reporting(scaled(0), 5, 100);
        let value = feed.get_value(100, 100, 5, false).expect("zero is allowed");
        let (a, b) = (Pubkey::new_unique(), Pubkey::new_unique());
        assert_eq!(resolve_oracle_winner(value, a, b).unwrap(), a);
    }

    #[test]
    fn get_value_rejects_insufficient_samples() {
        // 3 samples but min 5 → NotEnoughSamples (propagates as OracleWinnerNotInMatch).
        let feed = feed_reporting(scaled(1), 3, 100);
        assert!(feed.get_value(100, 100, 5, false).is_err());
    }
}
