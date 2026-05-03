use anchor_lang::prelude::*;
use anchor_lang::solana_program::keccak;
use anchor_spl::token::{self, Token, TokenAccount, Transfer};

use crate::constants::{BPS_DENOMINATOR, MATCH_SEED, PROTOCOL_FEE_BPS, TOURNAMENT_SEED, VAULT_SEED};
use crate::errors::BracketChainError;
use crate::events::{MatchReported, PlacementPayout, TournamentCompleted};
use crate::state::{
    MatchNode, MatchStatus, ProtocolConfig, Tournament, TournamentStatus,
};

#[derive(Accounts)]
pub struct ReportResult<'info> {
    #[account(mut, address = tournament.organizer @ BracketChainError::UnauthorizedAuthority)]
    pub organizer: Signer<'info>,

    #[account(
        mut,
        seeds = [
            TOURNAMENT_SEED,
            tournament.organizer.as_ref(),
            &keccak::hashv(&[tournament.name.as_bytes()]).0,
        ],
        bump = tournament.bump,
    )]
    pub tournament: Account<'info, Tournament>,

    #[account(
        mut,
        seeds = [
            MATCH_SEED,
            tournament.key().as_ref(),
            &[match_account.round],
            &match_account.match_index.to_le_bytes(),
        ],
        bump = match_account.bump,
        constraint = match_account.tournament == tournament.key()
            @ BracketChainError::InvalidMatchIndex,
    )]
    pub match_account: Account<'info, MatchNode>,

    /// Required for non-final matches; pass `None` when reporting the final.
    #[account(mut)]
    pub next_match: Option<Account<'info, MatchNode>>,

    #[account(
        seeds = [crate::constants::PROTOCOL_CONFIG_SEED],
        bump = protocol_config.bump,
    )]
    pub protocol_config: Account<'info, ProtocolConfig>,

    #[account(
        mut,
        seeds = [VAULT_SEED, tournament.key().as_ref()],
        bump = tournament.vault_bump,
        constraint = vault.key() == tournament.vault @ BracketChainError::InvalidVault,
    )]
    pub vault: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
}

pub(crate) fn handler<'info>(
    ctx: Context<'_, '_, '_, 'info, ReportResult<'info>>,
    winner: Pubkey,
    placements: Vec<Pubkey>,
) -> Result<()> {
    let tournament_key = ctx.accounts.tournament.key();

    require!(
        ctx.accounts.tournament.status == TournamentStatus::Active,
        BracketChainError::NotActive
    );

    let match_round = ctx.accounts.match_account.round;
    let match_idx = ctx.accounts.match_account.match_index;
    let player_a = ctx.accounts.match_account.player_a;
    let player_b = ctx.accounts.match_account.player_b;

    require!(
        ctx.accounts.match_account.status == MatchStatus::Active,
        BracketChainError::MatchAlreadyReported
    );
    require!(
        winner == player_a || winner == player_b,
        BracketChainError::NonParticipantWinner
    );

    let now = Clock::get()?.unix_timestamp;

    {
        let m = &mut ctx.accounts.match_account;
        m.winner = winner;
        m.status = MatchStatus::Completed;
    }

    ctx.accounts.tournament.matches_reported = ctx
        .accounts
        .tournament
        .matches_reported
        .checked_add(1)
        .ok_or(BracketChainError::ArithmeticOverflow)?;

    emit!(MatchReported {
        tournament: tournament_key,
        round: match_round,
        match_index: match_idx,
        winner,
        reported_at: now,
    });

    let bracket_size = ctx.accounts.tournament.bracket_size;
    let max_round = bracket_size.trailing_zeros() as u8;
    let is_final = match_round + 1 == max_round && match_idx == 0;

    if is_final {
        require!(
            ctx.accounts.next_match.is_none(),
            BracketChainError::InvalidMatchIndex
        );

        let (gross_pool, fee_amount, net_pool, placement_payouts) =
            distribute_prizes(&ctx, winner, player_a, player_b, &placements)?;

        let treasury_recipient = ctx.accounts.protocol_config.treasury;

        let tournament = &mut ctx.accounts.tournament;
        tournament.status = TournamentStatus::Completed;
        tournament.champion = winner;
        tournament.completed_at = now;

        emit!(TournamentCompleted {
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
        require!(
            placements.is_empty(),
            BracketChainError::InvalidPayoutPreset
        );
        advance_winner(
            &mut ctx.accounts.next_match,
            tournament_key,
            match_round,
            match_idx,
            winner,
        )?;
    }

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
/// `placement_payouts` includes only non-zero amounts in CPI-execution order
/// (place=1, 2, ...). The treasury fee is emitted separately via fee_amount.
fn distribute_prizes<'info>(
    ctx: &Context<'_, '_, '_, 'info, ReportResult<'info>>,
    winner: Pubkey,
    final_player_a: Pubkey,
    final_player_b: Pubkey,
    placements: &[Pubkey],
) -> Result<(u64, u64, u64, Vec<PlacementPayout>)> {
    let payout_preset = ctx.accounts.tournament.payout_preset;
    let placement_count = payout_preset.placement_count();

    require_eq!(
        placements.len(),
        placement_count,
        BracketChainError::InvalidPayoutPreset
    );
    require_eq!(
        ctx.remaining_accounts.len(),
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

    let organizer_key = ctx.accounts.tournament.organizer;
    let tournament_name = ctx.accounts.tournament.name.clone();
    let tournament_bump = ctx.accounts.tournament.bump;
    let token_mint = ctx.accounts.tournament.token_mint;

    let gross_pool = ctx.accounts.vault.amount;
    let fee_amount = (gross_pool as u128)
        .checked_mul(PROTOCOL_FEE_BPS as u128)
        .ok_or(BracketChainError::ArithmeticOverflow)?
        .checked_div(BPS_DENOMINATOR as u128)
        .ok_or(BracketChainError::ArithmeticOverflow)? as u64;
    let net_pool = gross_pool
        .checked_sub(fee_amount)
        .ok_or(BracketChainError::ArithmeticOverflow)?;

    let bps_table = payout_preset.basis_points();
    let bump_slice = [tournament_bump];
    let name_hash = keccak::hashv(&[tournament_name.as_bytes()]).0;
    let signer_seeds: &[&[&[u8]]] = &[&[
        TOURNAMENT_SEED,
        organizer_key.as_ref(),
        &name_hash,
        &bump_slice,
    ]];

    let mut placement_payouts: Vec<PlacementPayout> = Vec::with_capacity(placement_count);

    for i in 0..placement_count {
        let bps = bps_table[i];
        let amount = (net_pool as u128)
            .checked_mul(bps as u128)
            .ok_or(BracketChainError::ArithmeticOverflow)?
            .checked_div(BPS_DENOMINATOR as u128)
            .ok_or(BracketChainError::ArithmeticOverflow)? as u64;

        if amount == 0 {
            continue;
        }

        let ata_info = &ctx.remaining_accounts[i];
        validate_token_account(ata_info, &placements[i], &token_mint)?;

        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.vault.to_account_info(),
                    to: ata_info.clone(),
                    authority: ctx.accounts.tournament.to_account_info(),
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
        let treasury_ata = &ctx.remaining_accounts[placement_count];
        let treasury_wallet = ctx.accounts.protocol_config.treasury;
        validate_token_account(treasury_ata, &treasury_wallet, &token_mint)?;

        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.vault.to_account_info(),
                    to: treasury_ata.clone(),
                    authority: ctx.accounts.tournament.to_account_info(),
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
