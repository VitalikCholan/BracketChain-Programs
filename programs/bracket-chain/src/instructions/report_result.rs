use anchor_lang::prelude::*;
use anchor_spl::token::{Token, TokenAccount};

use crate::constants::{MATCH_SEED, TOURNAMENT_SEED, VAULT_SEED};
use crate::errors::BracketChainError;
use crate::instructions::settlement::finalize_match;
use crate::state::{MatchNode, ProtocolConfig, SettlementMode, Tournament};

#[derive(Accounts)]
pub struct ReportResult<'info> {
    #[account(mut, address = tournament.organizer @ BracketChainError::UnauthorizedAuthority)]
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
    // Boxed: V1.1 grew Tournament + ProtocolConfig; without heap-allocating
    // these, `try_accounts` overflows the SBF 4KB stack frame.
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
    // Boxed: V1.2 grew MatchNode (Oracle commitment + `expected_feed_hash`),
    // pushing `try_accounts` over the SBF 4KB stack frame. Deref-coercion keeps
    // the finalize_match call compatible.
    pub match_account: Box<Account<'info, MatchNode>>,

    /// Required for non-final matches; pass `None` when reporting the final.
    #[account(mut)]
    pub next_match: Option<Account<'info, MatchNode>>,

    #[account(
        seeds = [crate::constants::PROTOCOL_CONFIG_SEED],
        bump = protocol_config.bump,
    )]
    pub protocol_config: Box<Account<'info, ProtocolConfig>>,

    #[account(
        mut,
        seeds = [VAULT_SEED, tournament.key().as_ref()],
        bump = tournament.vault_bump,
        constraint = vault.key() == tournament.vault @ BracketChainError::InvalidVault,
    )]
    pub vault: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
}

/// OrganizerOnly settlement: the organizer reports the winner directly. The
/// shared `finalize_match` performs all validation, bracket advancement, and
/// (on the final match) prize distribution. The proposal envelope is never
/// touched in this mode.
pub(crate) fn handler<'info>(
    mut ctx: Context<'_, '_, '_, 'info, ReportResult<'info>>,
    winner: Pubkey,
    placements: Vec<Pubkey>,
) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    let accs = &mut ctx.accounts;

    // Settlement-mode gate (C-7). `report_result` is the **OrganizerOnly**
    // direct-report path — the organizer names the winner unilaterally. Reject
    // it for PlayerReported and Oracle modes, where results are trustless:
    // players/oracle `propose_result*` → `confirm_result`/`claim_result`, and
    // contested matches go to the arbitrator via `dispute_result` →
    // `resolve_dispute`. Allowing `report_result` there would let the organizer
    // bypass the dispute machinery and overwrite a finalized winner.
    //
    // Why a flat "OrganizerOnly only" check (not the v1.2-plan's "Oracle unless
    // disputed" carve-out):
    //   1. Closes a pre-existing Stage B gap — there was NO settlement gate
    //      here, so an organizer could already bypass PlayerReported settlement.
    //   2. The plan's carve-out is redundant: a disputed match is resolved by
    //      `resolve_dispute` (which also credits stats + emits `DisputeResolved`);
    //      routing it through `report_result` instead would skip both and leave
    //      the indexer inconsistent.
    //   3. It is also insufficient for the real failure it targeted — an Oracle
    //      match the feed never settles is stuck in Active with NO proposal, so
    //      it can't be disputed, so "unless disputed" never fires. That genuine
    //      escape-hatch belongs in a future arbitrator force-resolve ix, not in
    //      a winner-overwriting organizer path.
    require!(
        accs.tournament.settlement_mode == SettlementMode::OrganizerOnly,
        BracketChainError::SettlementModeMismatch
    );

    finalize_match(
        &mut accs.tournament,
        &mut accs.match_account,
        &mut accs.next_match,
        &mut accs.vault,
        &accs.protocol_config,
        &accs.token_program,
        ctx.remaining_accounts,
        winner,
        &placements,
        true, // organizer-signed (OrganizerOnly) — trusted to adjudicate placements (H-1)
        now,
    )?;

    Ok(())
}
