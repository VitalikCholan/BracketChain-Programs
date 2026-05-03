use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer};

use crate::constants::{TOURNAMENT_SEED, VAULT_SEED};
use crate::errors::BracketChainError;
use crate::events::{RefundIssued, TournamentCancelled};
use crate::state::{Participant, Tournament, TournamentStatus};

#[derive(Accounts)]
pub struct CancelTournament<'info> {
    /// Only required to be the organizer when flipping status to Cancelled.
    /// Once status == Cancelled, any signer can call to process refund chunks.
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

    /// Organizer's ATA in the tournament's token mint. Required only when an
    /// unrefunded `organizer_deposit > 0` is being processed in this call.
    /// Constraints (mint + owner) are validated in-handler so that callers
    /// processing later refund chunks may pass `None`.
    #[account(mut)]
    pub organizer_token_account: Option<Account<'info, TokenAccount>>,

    pub token_program: Program<'info, Token>,
}

pub(crate) fn handler<'info>(
    ctx: Context<'_, '_, '_, 'info, CancelTournament<'info>>,
) -> Result<()> {
    let tournament_key = ctx.accounts.tournament.key();
    let now = Clock::get()?.unix_timestamp;

    let status = ctx.accounts.tournament.status;
    require!(
        status == TournamentStatus::Registration
            || status == TournamentStatus::PendingBracketInit
            || status == TournamentStatus::Cancelled,
        BracketChainError::TournamentInProgress
    );

    // First call: only organizer can flip status to Cancelled.
    // Subsequent calls (status already Cancelled): any signer can process refunds.
    if status != TournamentStatus::Cancelled {
        require_keys_eq!(
            ctx.accounts.caller.key(),
            ctx.accounts.tournament.organizer,
            BracketChainError::UnauthorizedAuthority
        );
        ctx.accounts.tournament.status = TournamentStatus::Cancelled;
        emit!(TournamentCancelled {
            tournament: tournament_key,
            authority: ctx.accounts.caller.key(),
            cancelled_at: now,
        });
    }

    // Each participant occupies a pair of remaining_accounts: [pda, ata].
    require!(
        ctx.remaining_accounts.len() % 2 == 0,
        BracketChainError::RemainingAccountsMismatch
    );

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

    // Refund the organizer deposit once — gated on the flag for idempotency.
    // Allowed on any call (organizer or any signer post-flip) as long as the
    // organizer's ATA is supplied. Skipped silently when ATA is absent so
    // refund-chunking calls don't have to carry the organizer's ATA every time.
    if organizer_deposit > 0 && !deposit_refunded {
        if let Some(organizer_ata) = ctx.accounts.organizer_token_account.as_ref() {
            require_keys_eq!(
                organizer_ata.mint,
                token_mint,
                BracketChainError::InvalidTokenMint
            );
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
                tournament: tournament_key,
                wallet: organizer_key,
                amount: organizer_deposit,
            });
        }
    }

    for pair in ctx.remaining_accounts.chunks(2) {
        let participant_ai = &pair[0];
        let ata_ai = &pair[1];

        require_keys_eq!(
            *participant_ai.owner,
            *ctx.program_id,
            BracketChainError::InvalidMatchIndex
        );

        // Read participant.
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

        // Write back (disc + data).
        let mut data = participant_ai.try_borrow_mut_data()?;
        let mut writer: &mut [u8] = &mut data;
        participant.try_serialize(&mut writer)?;

        emit!(RefundIssued {
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
