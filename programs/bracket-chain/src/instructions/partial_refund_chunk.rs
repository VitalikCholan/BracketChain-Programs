use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer};

use crate::constants::{EVENT_VERSION_V1, TOURNAMENT_SEED, VAULT_SEED};
use crate::errors::BracketChainError;
use crate::events::RefundIssued;
use crate::state::{Participant, Tournament, TournamentStatus};

/// `partial_refund_chunk` — **permissionless** refund processing for a
/// partially-cancelled tournament (Stage E, E-3). Requires
/// `status == PartialCancelled` (set by `partial_cancel_tournament`).
///
/// **Policy A — full refund to all.** Every participant — alive or already
/// eliminated — is refunded their full `entry_fee`; the organizer recovers
/// only their deposit (no surplus). `losses` is deliberately NOT consulted:
/// cancelling must be a pure loss for the organizer, never a profit (an
/// "organizer keeps eliminated fees" rule was rejected — rake-abort hazard).
///
/// Chunked: `remaining_accounts` is pairs of `[participant_pda, ata]`,
/// ~10–24/call. Idempotent via `participant.refund_paid`; the organizer deposit
/// is gated by `organizer_deposit_refunded`. Mirrors `cancel_tournament`'s
/// refund loop, differing only in the accepted status.
#[derive(Accounts)]
pub struct PartialRefundChunk<'info> {
    #[account(mut)]
    pub caller: Signer<'info>,

    #[account(
        mut,
        seeds = [
            TOURNAMENT_SEED,
            tournament.organizer.as_ref(),
            tournament.name.as_bytes(),
        ],
        bump = tournament.bump,
    )]
    pub tournament: Account<'info, Tournament>,

    #[account(
        mut,
        seeds = [VAULT_SEED, tournament.key().as_ref()],
        bump = tournament.vault_bump,
        constraint = vault.key() == tournament.vault @ BracketChainError::InvalidVault,
    )]
    pub vault: Account<'info, TokenAccount>,

    /// Organizer's ATA — required only on the call that returns the deposit
    /// (`organizer_deposit > 0 && !organizer_deposit_refunded`). Validated
    /// in-handler so later refund-only chunks may pass `None`.
    #[account(mut)]
    pub organizer_token_account: Option<Account<'info, TokenAccount>>,

    pub token_program: Program<'info, Token>,
    // remaining_accounts: pairs of [participant_pda, ata] to refund.
}

pub(crate) fn handler<'info>(
    ctx: Context<'_, '_, '_, 'info, PartialRefundChunk<'info>>,
) -> Result<()> {
    require!(
        ctx.accounts.tournament.status == TournamentStatus::PartialCancelled,
        BracketChainError::TournamentInProgress
    );

    require!(
        ctx.remaining_accounts.len() % 2 == 0,
        BracketChainError::RemainingAccountsMismatch
    );

    let tournament_key = ctx.accounts.tournament.key();
    let entry_fee = ctx.accounts.tournament.entry_fee;
    let token_mint = ctx.accounts.tournament.token_mint;
    let organizer_key = ctx.accounts.tournament.organizer;
    let tournament_name = ctx.accounts.tournament.name.clone();
    let tournament_bump = ctx.accounts.tournament.bump;
    let organizer_deposit = ctx.accounts.tournament.organizer_deposit;
    let deposit_refunded = ctx.accounts.tournament.organizer_deposit_refunded;

    let bump_slice = [tournament_bump];
    let signer_seeds: &[&[&[u8]]] = &[&[
        TOURNAMENT_SEED,
        organizer_key.as_ref(),
        tournament_name.as_bytes(),
        &bump_slice,
    ]];

    // Return the organizer deposit once — gated for idempotency. Skipped
    // silently when the ATA is absent so refund-only chunks need not carry it.
    if organizer_deposit > 0 && !deposit_refunded {
        if let Some(organizer_ata) = ctx.accounts.organizer_token_account.as_ref() {
            require_keys_eq!(organizer_ata.mint, token_mint, BracketChainError::InvalidTokenMint);
            require_keys_eq!(
                organizer_ata.owner,
                organizer_key,
                BracketChainError::UnauthorizedAuthority
            );
            token::transfer(
                CpiContext::new_with_signer(
                    ctx.accounts.token_program.to_account_info(),
                    Transfer {
                        from: ctx.accounts.vault.to_account_info(),
                        to: organizer_ata.to_account_info(),
                        authority: ctx.accounts.tournament.to_account_info(),
                    },
                    signer_seeds,
                ),
                organizer_deposit,
            )?;
            ctx.accounts.tournament.organizer_deposit_refunded = true;
            emit!(RefundIssued {
                event_version: EVENT_VERSION_V1,
                tournament: tournament_key,
                wallet: organizer_key,
                amount: organizer_deposit,
            });
        }
    }

    // Refund every participant their full entry fee (Policy A — no `losses`
    // filter). Idempotent via `refund_paid`.
    for pair in ctx.remaining_accounts.chunks(2) {
        let participant_ai = &pair[0];
        let ata_ai = &pair[1];

        require_keys_eq!(
            *participant_ai.owner,
            *ctx.program_id,
            BracketChainError::InvalidMatchIndex
        );

        let mut participant: Participant = {
            let data = participant_ai.try_borrow_data()?;
            let mut buf: &[u8] = &data;
            Participant::try_deserialize(&mut buf)?
        };

        require_keys_eq!(
            participant.tournament,
            tournament_key,
            BracketChainError::InvalidMatchIndex
        );

        if participant.refund_paid {
            continue;
        }

        validate_token_account(ata_ai, &participant.wallet, &token_mint)?;

        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.vault.to_account_info(),
                    to: ata_ai.clone(),
                    authority: ctx.accounts.tournament.to_account_info(),
                },
                signer_seeds,
            ),
            entry_fee,
        )?;

        participant.refund_paid = true;
        {
            let mut data = participant_ai.try_borrow_mut_data()?;
            let mut writer: &mut [u8] = &mut data;
            participant.try_serialize(&mut writer)?;
        }

        emit!(RefundIssued {
            event_version: EVENT_VERSION_V1,
            tournament: tournament_key,
            wallet: participant.wallet,
            amount: entry_fee,
        });
    }

    Ok(())
}

fn validate_token_account(
    ai: &AccountInfo,
    expected_owner: &Pubkey,
    expected_mint: &Pubkey,
) -> Result<()> {
    require_keys_eq!(*ai.owner, anchor_spl::token::ID, BracketChainError::InvalidTokenMint);
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
