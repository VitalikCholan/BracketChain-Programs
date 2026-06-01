//! Shared match-finalization logic.
//!
//! Both `report_result` (OrganizerOnly settlement) and the player-reported /
//! Oracle finalize paths (`confirm_result` / `claim_result` / `resolve_dispute`
//! / `force_claim_disputed`) converge here so prize distribution and bracket
//! advancement live in exactly one place. The functions take raw account refs
//! rather than a specific `Context`, so any instruction can call them.

use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer};

use crate::constants::{
    BPS_DENOMINATOR, EVENT_VERSION_V1, PROTOCOL_FEE_BPS, TOURNAMENT_SEED,
};
use crate::errors::BracketChainError;
use crate::events::{MatchReported, PlacementPayout, RefundIssued, TournamentCompleted};
use crate::state::{
    MatchNode, MatchStatus, Participant, ProtocolConfig, Tournament, TournamentStatus,
};

/// Marks `match_account` completed for `winner`, emits `MatchReported`, then
/// either advances the winner into the parent slot or — on the final match —
/// refunds the organizer deposit, distributes the prize pool, and completes the
/// tournament. Validation common to every settlement path (tournament Active,
/// match Active, winner ∈ {player_a, player_b}) is performed here.
///
/// `placements` + `remaining_accounts` are only consulted on the final match
/// (same contract as `report_result`); pass empty / `&[]` otherwise.
#[allow(clippy::too_many_arguments)]
pub fn finalize_match<'info>(
    tournament: &mut Account<'info, Tournament>,
    match_account: &mut Account<'info, MatchNode>,
    next_match: &mut Option<Account<'info, MatchNode>>,
    vault: &mut Account<'info, TokenAccount>,
    organizer_token_account: &Option<Account<'info, TokenAccount>>,
    protocol_config: &ProtocolConfig,
    token_program: &Program<'info, Token>,
    remaining_accounts: &[AccountInfo<'info>],
    winner: Pubkey,
    placements: &[Pubkey],
    // H-1: `true` only for trusted-signer paths (organizer `report_result`,
    // arbitrator `resolve_dispute` / `settle_final`). Permissionless and
    // counterparty paths pass `false` and may only finalize WinnerTakesAll
    // finals (where placements carry no organizer-trusted slots).
    placements_trusted: bool,
    now: i64,
) -> Result<()> {
    require!(
        tournament.status == TournamentStatus::Active,
        BracketChainError::NotActive
    );

    let tournament_key = tournament.key();
    let bracket = match_account.bracket;
    let match_round = match_account.round;
    let match_idx = match_account.match_index;
    let player_a = match_account.player_a;
    let player_b = match_account.player_b;

    require!(
        match_account.status == MatchStatus::Active,
        BracketChainError::MatchAlreadyReported
    );
    require!(
        winner == player_a || winner == player_b,
        BracketChainError::NonParticipantWinner
    );

    match_account.winner = winner;
    match_account.status = MatchStatus::Completed;

    tournament.matches_reported = tournament
        .matches_reported
        .checked_add(1)
        .ok_or(BracketChainError::ArithmeticOverflow)?;

    emit!(MatchReported {
        event_version: EVENT_VERSION_V1,
        tournament: tournament_key,
        bracket,
        round: match_round,
        match_index: match_idx,
        winner,
        reported_at: now,
    });

    let bracket_size = tournament.bracket_size;
    let max_round = bracket_size.trailing_zeros() as u8;
    let is_final = match_round + 1 == max_round && match_idx == 0;

    if is_final {
        // H-1: single-elim has no 3rd-place match, so `placements[2..]` are
        // unconstrained on-chain (only `placements[0]`/`[1]` are validated in
        // `distribute_prizes`). Multi-placement presets (Standard/Deep/Custom)
        // therefore require a trusted signer to adjudicate the lower placements;
        // permissionless / counterparty callers are limited to WinnerTakesAll
        // (`placement_count == 1`). This lifts decision-2a from an off-chain
        // cron convention into an on-chain invariant.
        require!(
            placements_trusted || tournament.payout_preset.placement_count() <= 1,
            BracketChainError::UntrustedMultiPlacementFinal
        );

        require!(
            next_match.is_none(),
            BracketChainError::InvalidMatchIndex
        );

        // Variant A: refund the organizer's deposit before computing the
        // prize-pool basis. Invariantly not-yet-refunded here (cancel is gated
        // pre-start; final-match requires Active).
        refund_organizer_deposit(tournament, vault, organizer_token_account, token_program)?;

        let (gross_pool, fee_amount, net_pool, placement_payouts) = distribute_prizes(
            tournament,
            protocol_config,
            vault,
            token_program,
            remaining_accounts,
            winner,
            player_a,
            player_b,
            placements,
        )?;

        let treasury_recipient = protocol_config.treasury;

        tournament.status = TournamentStatus::Completed;
        tournament.champion = winner;
        tournament.completed_at = now;

        emit!(TournamentCompleted {
            event_version: EVENT_VERSION_V1,
            tournament: tournament_key,
            champion: winner,
            gross_pool,
            fee_amount,
            net_pool,
            completed_at: now,
            placement_payouts,
            treasury_recipient,
        });
    } else {
        require!(placements.is_empty(), BracketChainError::InvalidPayoutPreset);
        advance_winner(next_match, tournament_key, match_round, match_idx, winner)?;
    }

    Ok(())
}

/// Credits a win to the match winner and a loss to the loser. Scores
/// (`points_for` / `points_against`) are left untouched in V1 — the
/// player-reported flow carries no score, only a winner; scores arrive with the
/// V1.2 Oracle. `report_result` (OrganizerOnly) does not call this, so its
/// tournaments leave participant stats at zero (acceptable for V1).
pub fn credit_match_stats<'info>(
    participant_a: &mut Account<'info, Participant>,
    participant_b: &mut Account<'info, Participant>,
    winner: Pubkey,
) -> Result<()> {
    let (win_p, lose_p) = if participant_a.wallet == winner {
        (participant_a, participant_b)
    } else {
        (participant_b, participant_a)
    };
    win_p.wins = win_p.wins.saturating_add(1);
    lose_p.losses = lose_p.losses.saturating_add(1);
    Ok(())
}

fn refund_organizer_deposit<'info>(
    tournament: &mut Account<'info, Tournament>,
    vault: &mut Account<'info, TokenAccount>,
    organizer_token_account: &Option<Account<'info, TokenAccount>>,
    token_program: &Program<'info, Token>,
) -> Result<()> {
    let organizer_deposit = tournament.organizer_deposit;
    if organizer_deposit == 0 || tournament.organizer_deposit_refunded {
        return Ok(());
    }

    let organizer_ata = organizer_token_account
        .as_ref()
        .ok_or(error!(BracketChainError::InvalidVault))?;

    let tournament_key = tournament.key();
    let organizer_key = tournament.organizer;
    let tournament_name = tournament.name.clone();
    let bump_slice = [tournament.bump];
    let signer_seeds: &[&[&[u8]]] = &[&[
        TOURNAMENT_SEED,
        organizer_key.as_ref(),
        tournament_name.as_bytes(),
        &bump_slice,
    ]];

    token::transfer(
        CpiContext::new_with_signer(
            token_program.to_account_info(),
            Transfer {
                from: vault.to_account_info(),
                to: organizer_ata.to_account_info(),
                authority: tournament.to_account_info(),
            },
            signer_seeds,
        ),
        organizer_deposit,
    )?;

    tournament.organizer_deposit_refunded = true;
    vault.reload()?;

    emit!(RefundIssued {
        event_version: EVENT_VERSION_V1,
        tournament: tournament_key,
        wallet: organizer_key,
        amount: organizer_deposit,
    });

    Ok(())
}

fn advance_winner<'info>(
    next_match_opt: &mut Option<Account<'info, MatchNode>>,
    tournament_key: Pubkey,
    match_round: u8,
    match_idx: u16,
    winner: Pubkey,
) -> Result<()> {
    let next_match = next_match_opt
        .as_mut()
        .ok_or(error!(BracketChainError::InvalidMatchIndex))?;

    require_keys_eq!(
        next_match.tournament,
        tournament_key,
        BracketChainError::InvalidMatchIndex
    );
    require_eq!(
        next_match.round,
        match_round + 1,
        BracketChainError::InvalidMatchIndex
    );
    require_eq!(
        next_match.match_index,
        match_idx / 2,
        BracketChainError::InvalidMatchIndex
    );

    let is_left_child = match_idx % 2 == 0;
    if is_left_child {
        next_match.player_a = winner;
    } else {
        next_match.player_b = winner;
    }

    if next_match.player_a != Pubkey::default() && next_match.player_b != Pubkey::default() {
        next_match.status = MatchStatus::Active;
    }

    Ok(())
}

/// Returns (gross_pool, fee_amount, net_pool, placement_payouts).
/// `placement_payouts` includes only non-zero amounts in CPI-execution order.
#[allow(clippy::too_many_arguments)]
fn distribute_prizes<'info>(
    tournament: &Account<'info, Tournament>,
    protocol_config: &ProtocolConfig,
    vault: &Account<'info, TokenAccount>,
    token_program: &Program<'info, Token>,
    remaining_accounts: &[AccountInfo<'info>],
    winner: Pubkey,
    final_player_a: Pubkey,
    final_player_b: Pubkey,
    placements: &[Pubkey],
) -> Result<(u64, u64, u64, Vec<PlacementPayout>)> {
    let payout_preset = tournament.payout_preset;
    let placement_count = payout_preset.placement_count();

    require_eq!(
        placements.len(),
        placement_count,
        BracketChainError::InvalidPayoutPreset
    );
    require_eq!(
        remaining_accounts.len(),
        placement_count + 1,
        BracketChainError::RemainingAccountsMismatch
    );

    require_keys_eq!(placements[0], winner, BracketChainError::NonParticipantWinner);
    if placements.len() >= 2 {
        let runner_up = if winner == final_player_a {
            final_player_b
        } else {
            final_player_a
        };
        require_keys_eq!(
            placements[1],
            runner_up,
            BracketChainError::NonParticipantWinner
        );
    }

    let organizer_key = tournament.organizer;
    let tournament_name = tournament.name.clone();
    let token_mint = tournament.token_mint;

    let gross_pool = vault.amount;
    let fee_amount = (gross_pool as u128)
        .checked_mul(PROTOCOL_FEE_BPS as u128)
        .ok_or(BracketChainError::ArithmeticOverflow)?
        .checked_div(BPS_DENOMINATOR as u128)
        .ok_or(BracketChainError::ArithmeticOverflow)? as u64;
    let net_pool = gross_pool
        .checked_sub(fee_amount)
        .ok_or(BracketChainError::ArithmeticOverflow)?;

    let bps_table = payout_preset.basis_points();
    let bump_slice = [tournament.bump];
    let signer_seeds: &[&[&[u8]]] = &[&[
        TOURNAMENT_SEED,
        organizer_key.as_ref(),
        tournament_name.as_bytes(),
        &bump_slice,
    ]];

    // Champion (place 1) absorbs the floor-division remainder so the amounts
    // sum to exactly `net_pool` and the vault drains to zero — otherwise the
    // few-base-unit dust would block `close_tournament`'s `vault.amount == 0`
    // root-close guard forever for any non-WinnerTakesAll preset (Medium-1).
    // Refund paths (cancel / partial) are exact and unaffected. Pure split is
    // unit-tested in `split_tests` below.
    let amounts = split_placements(net_pool, &bps_table, placement_count)?;

    let mut placement_payouts: Vec<PlacementPayout> = Vec::with_capacity(placement_count);

    for i in 0..placement_count {
        let amount = amounts[i];
        if amount == 0 {
            continue;
        }

        let ata_info = &remaining_accounts[i];
        validate_token_account(ata_info, &placements[i], &token_mint)?;

        token::transfer(
            CpiContext::new_with_signer(
                token_program.to_account_info(),
                Transfer {
                    from: vault.to_account_info(),
                    to: ata_info.clone(),
                    authority: tournament.to_account_info(),
                },
                signer_seeds,
            ),
            amount,
        )?;

        placement_payouts.push(PlacementPayout {
            place: (i + 1) as u8,
            recipient: placements[i],
            amount,
        });
    }

    if fee_amount > 0 {
        let treasury_ata = &remaining_accounts[placement_count];
        let treasury_wallet = protocol_config.treasury;
        validate_token_account(treasury_ata, &treasury_wallet, &token_mint)?;

        token::transfer(
            CpiContext::new_with_signer(
                token_program.to_account_info(),
                Transfer {
                    from: vault.to_account_info(),
                    to: treasury_ata.clone(),
                    authority: tournament.to_account_info(),
                },
                signer_seeds,
            ),
            fee_amount,
        )?;
    }

    Ok((gross_pool, fee_amount, net_pool, placement_payouts))
}

fn validate_token_account(
    ai: &AccountInfo,
    expected_owner: &Pubkey,
    expected_mint: &Pubkey,
) -> Result<()> {
    require_keys_eq!(
        *ai.owner,
        anchor_spl::token::ID,
        BracketChainError::InvalidTokenMint
    );
    let data = ai.try_borrow_data()?;
    require!(data.len() >= 165, BracketChainError::InvalidTokenMint);
    let mint = Pubkey::try_from(&data[0..32])
        .map_err(|_| error!(BracketChainError::InvalidTokenMint))?;
    let owner = Pubkey::try_from(&data[32..64])
        .map_err(|_| error!(BracketChainError::InvalidTokenMint))?;
    require_keys_eq!(mint, *expected_mint, BracketChainError::InvalidTokenMint);
    require_keys_eq!(owner, *expected_owner, BracketChainError::InvalidTreasury);
    Ok(())
}

/// Splits `net_pool` across the first `placement_count` slots of `bps_table`,
/// flooring each and giving the champion (slot 0) the remainder so the result
/// sums to **exactly** `net_pool`. Pure (no accounts / CPI) for unit-testing;
/// keeps the vault dust-free so `close_tournament` can close it (Medium-1).
fn split_placements(net_pool: u64, bps_table: &[u16], placement_count: usize) -> Result<Vec<u64>> {
    let mut amounts: Vec<u64> = Vec::with_capacity(placement_count);
    for &bps in bps_table.iter().take(placement_count) {
        let amount = (net_pool as u128)
            .checked_mul(bps as u128)
            .ok_or(BracketChainError::ArithmeticOverflow)?
            .checked_div(BPS_DENOMINATOR as u128)
            .ok_or(BracketChainError::ArithmeticOverflow)? as u64;
        amounts.push(amount);
    }
    if placement_count > 0 {
        let mut lower_sum: u64 = 0;
        for &a in amounts.iter().skip(1) {
            lower_sum = lower_sum
                .checked_add(a)
                .ok_or(BracketChainError::ArithmeticOverflow)?;
        }
        // Lower floors sum to ≤ net_pool (bps sum to ≤ 10_000), so the champion
        // share stays ≥ its own floored amount.
        amounts[0] = net_pool
            .checked_sub(lower_sum)
            .ok_or(BracketChainError::ArithmeticOverflow)?;
    }
    Ok(amounts)
}

#[cfg(test)]
mod split_tests {
    use super::split_placements;
    use crate::constants::{PAYOUT_DEEP, PAYOUT_STANDARD, PAYOUT_WTA};

    // The Medium-1 invariant: a full distribution leaves zero dust in the vault.
    fn assert_sums_to_net(net: u64, bps: &[u16], count: usize) -> Vec<u64> {
        let a = split_placements(net, bps, count).unwrap();
        assert_eq!(a.iter().sum::<u64>(), net, "amounts must sum to the net pool");
        a
    }

    #[test]
    fn wta_is_exact_and_unchanged() {
        assert_eq!(assert_sums_to_net(1_234_567, &PAYOUT_WTA, 1), vec![1_234_567]);
    }

    #[test]
    fn standard_dusty_net_goes_entirely_to_champion() {
        // net = 101 → floors 60/25/15 = 100; champion absorbs the 1-unit dust.
        assert_eq!(assert_sums_to_net(101, &PAYOUT_STANDARD, 3), vec![61, 25, 15]);
    }

    #[test]
    fn standard_clean_net_matches_plain_floor() {
        // net divisible → champion share equals the plain floor (no change vs
        // the pre-fix code on clean pools — keeps existing payout tests valid).
        assert_eq!(
            assert_sums_to_net(3_860_000, &PAYOUT_STANDARD, 3),
            vec![2_316_000, 965_000, 579_000]
        );
    }

    #[test]
    fn custom_indivisible_bps_leaves_no_dust() {
        // Custom [3334,3333,3333] with an awkward net: plain floors lose 2
        // units; the champion absorbs them so the vault drains to zero.
        let bps = [3334u16, 3333, 3333, 0, 0, 0, 0, 0];
        let a = assert_sums_to_net(1_286_666, &bps, 3);
        assert_eq!(a, vec![428_976, 428_845, 428_845]);
        assert!(a[0] >= 428_974, "champion never receives less than its floor");
    }

    #[test]
    fn deep_clean_net_is_exact() {
        assert_sums_to_net(123_520_000, &PAYOUT_DEEP, 7);
    }

    #[test]
    fn tiny_net_all_remainder_to_champion() {
        assert_eq!(assert_sums_to_net(2, &PAYOUT_STANDARD, 3), vec![2, 0, 0]);
    }
}
