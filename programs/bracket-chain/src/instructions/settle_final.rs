use anchor_lang::prelude::*;
use anchor_spl::token::{Token, TokenAccount};

use crate::constants::{
    EVENT_VERSION_V1, MATCH_SEED, PARTICIPANT_SEED, TOURNAMENT_SEED, VAULT_SEED,
};
use crate::errors::BracketChainError;
use crate::events::FinalSettled;
use crate::instructions::settlement::{credit_match_stats, finalize_match};
use crate::state::{MatchNode, Participant, ProposalSource, ProtocolConfig, Tournament};

/// Accounts for `settle_final` — the **trusted-signer** finalize path for a
/// multi-placement (non-`WinnerTakesAll`) final. Mirrors `PermissionlessFinalize`
/// (`claim_result`) account-for-account, swapping the permissionless `payer` for
/// the tournament's `arbitrator`. The arbitrator adjudicates placements 3..N
/// among the semifinal losers; the **winner is still read from the match's
/// trustless proposal** (`proposed_winner`) — the arbitrator cannot change it.
#[derive(Accounts)]
pub struct SettleFinal<'info> {
    /// The tournament's arbitrator (defaults to the organizer at create-time;
    /// Squads-multisig reassignment is V1.3). The single point of trust — only
    /// it may adjudicate the lower placements of a non-WTA final.
    #[account(mut, address = tournament.arbitrator @ BracketChainError::UnauthorizedAuthority)]
    pub arbitrator: Signer<'info>,

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
    // Boxed: V1.2 grew MatchNode, pushing `try_accounts` over the SBF 4KB stack
    // frame. Deref-coercion keeps the finalize_match call compatible.
    pub match_account: Box<Account<'info, MatchNode>>,

    /// Final-match only — pass `None`. `finalize_match` requires `next_match`
    /// absent on the final and rejects a non-final that carries placements.
    #[account(mut)]
    pub next_match: Option<Account<'info, MatchNode>>,

    #[account(mut, seeds = [PARTICIPANT_SEED, tournament.key().as_ref(), match_account.player_a.as_ref()], bump = participant_a.bump)]
    pub participant_a: Box<Account<'info, Participant>>,

    #[account(mut, seeds = [PARTICIPANT_SEED, tournament.key().as_ref(), match_account.player_b.as_ref()], bump = participant_b.bump)]
    pub participant_b: Box<Account<'info, Participant>>,

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

/// Arbitrator-signed settlement of a non-WTA final (H-1 fix). Preconditions
/// mirror `claim_result` exactly — an **undisputed** proposal whose dispute
/// window has elapsed — so the winner is the same trustless `proposed_winner`
/// the players/oracle established; only the lower placements are the
/// arbitrator's call. Disputed finals route through `resolve_dispute` (where the
/// arbitrator legitimately picks the winner); `WinnerTakesAll` finals stay
/// permissionlessly claimable via `claim_result`. This is the trusted sibling of
/// `claim_result`, not a winner-overriding admin path (§4 / decision-2a).
pub(crate) fn handler<'info>(
    mut ctx: Context<'_, '_, '_, 'info, SettleFinal<'info>>,
    placements: Vec<Pubkey>,
) -> Result<()> {
    let arbitrator = ctx.accounts.arbitrator.key();
    let accs = &mut ctx.accounts;
    let now = Clock::get()?.unix_timestamp;

    require!(
        accs.match_account.proposal_source != ProposalSource::None,
        BracketChainError::NoProposal
    );
    require!(!accs.match_account.disputed, BracketChainError::ProposalDisputed);
    require!(
        now >= accs.match_account.claim_deadline,
        BracketChainError::ClaimWindowNotElapsed
    );

    // Winner is pinned to the proposal — the arbitrator only adjudicates
    // placements, never the result.
    let winner = accs.match_account.proposed_winner;
    let (tournament_key, bracket, round, match_index) = (
        accs.match_account.tournament,
        accs.match_account.bracket,
        accs.match_account.round,
        accs.match_account.match_index,
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
        true, // arbitrator-signed — trusted to adjudicate placements (H-1)
        now,
    )?;

    credit_match_stats(&mut accs.participant_a, &mut accs.participant_b, winner)?;

    emit!(FinalSettled {
        event_version: EVENT_VERSION_V1,
        tournament: tournament_key,
        bracket,
        round,
        match_index,
        arbitrator,
        winner,
        settled_at: now,
    });

    Ok(())
}
