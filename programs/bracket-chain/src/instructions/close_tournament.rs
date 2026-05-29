use anchor_lang::prelude::*;
use anchor_lang::system_program;
use anchor_spl::token::{self, CloseAccount, Token, TokenAccount};

use crate::constants::{EVENT_VERSION_V1, TOURNAMENT_SEED, VAULT_SEED};
use crate::errors::BracketChainError;
use crate::events::TournamentClosed;
use crate::state::{Tournament, TournamentStatus};

/// `close_tournament` — **permissionless** rent reclaim for a terminal
/// tournament (Stage D, D-3, gate G7). Closes child PDAs (MatchNode +
/// Participant) passed in `remaining_accounts`, ~10 per call, and — on the
/// final call (`close_root = true`, vault empty) — the vault token account +
/// the Tournament PDA itself. **All** reclaimed rent flows to the original
/// organizer (enforced by the `organizer` address constraint), never to the
/// cron payer who signs the tx.
///
/// Idempotent: an already-closed account has zero lamports and is skipped, so a
/// driver may safely re-send a chunk. Closing the root before every child is
/// closed would orphan the leftover child rent (a loss to the organizer only),
/// so the D-4 driver closes all children first, then sends one `close_root`
/// call.
#[derive(Accounts)]
pub struct CloseTournament<'info> {
    /// Permissionless — anyone may trigger cleanup; the `cleanup-payer` cron
    /// signs in practice. Rent never goes here (see `organizer`).
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

    /// Rent destination for every closed account. Pinned to the organizer.
    #[account(
        mut,
        address = tournament.organizer @ BracketChainError::UnauthorizedAuthority,
    )]
    pub organizer: SystemAccount<'info>,

    #[account(
        mut,
        seeds = [VAULT_SEED, tournament.key().as_ref()],
        bump = tournament.vault_bump,
        constraint = vault.key() == tournament.vault @ BracketChainError::InvalidVault,
    )]
    pub vault: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
    // remaining_accounts: child PDAs (MatchNode | Participant) to close.
}

pub(crate) fn handler<'info>(
    ctx: Context<'_, '_, '_, 'info, CloseTournament<'info>>,
    close_root: bool,
) -> Result<()> {
    let tournament_key = ctx.accounts.tournament.key();

    // Terminal-only. (Stage E adds `PartialCancelled`; extend this set then.)
    let status = ctx.accounts.tournament.status;
    require!(
        status == TournamentStatus::Completed || status == TournamentStatus::Cancelled,
        BracketChainError::TournamentInProgress
    );

    let program_id = ctx.program_id;
    let organizer_ai = ctx.accounts.organizer.to_account_info();

    // ── Close child PDAs (MatchNode | Participant) ──────────────────────────
    // Both structs carry `tournament: Pubkey` as their first field, so a child
    // of THIS tournament has `data[8..40] == tournament_key`. That + program
    // ownership is enough to authorize the close (the Tournament PDA's first
    // field is `organizer`, and the vault is token-program-owned, so neither
    // can sneak through here).
    let mut accounts_closed: u32 = 0;
    for child in ctx.remaining_accounts.iter() {
        // Idempotent: already closed.
        if child.lamports() == 0 {
            continue;
        }
        require_keys_eq!(*child.owner, *program_id, BracketChainError::InvalidMatchIndex);
        {
            let data = child.try_borrow_data()?;
            require!(data.len() >= 40, BracketChainError::InvalidMatchIndex);
            let embedded = Pubkey::try_from(&data[8..40])
                .map_err(|_| error!(BracketChainError::InvalidMatchIndex))?;
            require_keys_eq!(embedded, tournament_key, BracketChainError::InvalidMatchIndex);
        }
        close_account_info(child, &organizer_ai)?;
        accounts_closed += 1;
    }

    // ── Final call: close the vault + the Tournament PDA itself ─────────────
    if close_root {
        require!(
            ctx.accounts.vault.amount == 0,
            BracketChainError::TournamentInProgress
        );

        let organizer_key = ctx.accounts.tournament.organizer;
        let tournament_name = ctx.accounts.tournament.name.clone();
        let bump_slice = [ctx.accounts.tournament.bump];
        let signer_seeds: &[&[&[u8]]] = &[&[
            TOURNAMENT_SEED,
            organizer_key.as_ref(),
            tournament_name.as_bytes(),
            &bump_slice,
        ]];

        // Close the (empty) vault token account → rent to organizer.
        token::close_account(CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            CloseAccount {
                account: ctx.accounts.vault.to_account_info(),
                destination: organizer_ai.clone(),
                authority: ctx.accounts.tournament.to_account_info(),
            },
            signer_seeds,
        ))?;

        // Close the Tournament PDA. `Account<T>::close` moves lamports to the
        // destination and re-assigns the account to the system program, so
        // Anchor's exit routine skips re-serializing it.
        ctx.accounts
            .tournament
            .close(organizer_ai.clone())?;
    }

    emit!(TournamentClosed {
        event_version: EVENT_VERSION_V1,
        tournament: tournament_key,
        accounts_closed,
        root_closed: close_root,
    });

    Ok(())
}

/// Manually close a raw program-owned account: drain its lamports to `dest`,
/// zero its data, and re-assign it to the system program. Mirrors the
/// Anchor-internal close used for named accounts.
fn close_account_info<'info>(
    acct: &AccountInfo<'info>,
    dest: &AccountInfo<'info>,
) -> Result<()> {
    let lamports = acct.lamports();
    **dest.try_borrow_mut_lamports()? = dest
        .lamports()
        .checked_add(lamports)
        .ok_or(BracketChainError::ArithmeticOverflow)?;
    **acct.try_borrow_mut_lamports()? = 0;

    acct.assign(&system_program::ID);
    acct.realloc(0, false)?;
    Ok(())
}
