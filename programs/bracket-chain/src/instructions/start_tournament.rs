use anchor_lang::prelude::*;
use anchor_lang::solana_program::sysvar::slot_hashes;
use anchor_lang::system_program;
use solana_keccak_hasher as keccak;

use crate::constants::{EVENT_VERSION_V1, MATCH_SEED, MIN_PARTICIPANTS};
use crate::errors::BracketChainError;
use crate::events::TournamentStarted;
use crate::seeding::{round0_expected, seed_permutation};
use crate::state::{
    MatchNode, MatchStatus, Participant, SettlementMode, Tournament, TournamentStatus,
};

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct MatchInitDescriptor {
    /// Bracket lane — always `0` for single-elimination (V1). Part of the PDA
    /// seed (C9). See `MatchNode::bracket`.
    pub bracket: u8,
    pub round: u8,
    pub match_index: u16,
    pub bump: u8,
    pub player_a: Pubkey,
    pub player_b: Pubkey,
    pub bye: bool,
}

#[derive(Accounts)]
pub struct StartTournament<'info> {
    #[account(mut, address = tournament.organizer @ BracketChainError::UnauthorizedAuthority)]
    pub organizer: Signer<'info>,

    #[account(
        mut,
        seeds = [
            crate::constants::TOURNAMENT_SEED,
            tournament.organizer.as_ref(),
            tournament.name.as_bytes(),
        ],
        bump = tournament.bump,
    )]
    pub tournament: Account<'info, Tournament>,

    /// CHECK: Validated by address constraint to be the SlotHashes sysvar.
    /// Read manually because deserializing the full Vec is expensive.
    #[account(address = slot_hashes::ID)]
    pub slot_hashes: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}

pub(crate) fn handler<'info>(
    ctx: Context<'_, '_, '_, 'info, StartTournament<'info>>,
    descriptors: Vec<MatchInitDescriptor>,
) -> Result<()> {
    // Layout: the first `descriptors.len()` remaining accounts are the MatchNode
    // PDAs to create (1:1 with descriptors). For VRF-seeded (non-OrganizerOnly)
    // tournaments, the tail carries the `Participant` accounts proving each
    // placed player matches the seed-derived permutation — consumed by a cursor
    // in the loop below (H-2). OrganizerOnly passes no tail.
    require!(
        ctx.remaining_accounts.len() >= descriptors.len(),
        BracketChainError::RemainingAccountsMismatch
    );

    let tournament_key = ctx.accounts.tournament.key();

    // First chunk: capture seed_hash + compute bracket dimensions.
    if ctx.accounts.tournament.status == TournamentStatus::Registration {
        require!(
            ctx.accounts.tournament.participant_count >= MIN_PARTICIPANTS,
            BracketChainError::MinParticipantsNotMet
        );

        // Bracket seed source (B-5). If the organizer bound a Switchboard
        // randomness account via `request_seed`, the VRF seed is mandatory and
        // must already be revealed — `seed_hash` was written by `reveal_seed`,
        // so we leave it untouched. Otherwise (opt-in fallback) we derive the
        // seed from the SlotHashes sysvar as before.
        if ctx.accounts.tournament.vrf_randomness_account != Pubkey::default() {
            require!(
                ctx.accounts.tournament.seed_revealed,
                BracketChainError::SeedNotRevealed
            );
        } else {
            // H-2c: non-OrganizerOnly tournaments MUST be VRF-seeded. The
            // validator-influenceable SlotHashes fallback is only acceptable for
            // trust-mode (OrganizerOnly) brackets, where the organizer is
            // already the result authority — there, seeding manipulation buys
            // nothing it couldn't already do.
            require!(
                ctx.accounts.tournament.settlement_mode == SettlementMode::OrganizerOnly,
                BracketChainError::SeedNotRevealed
            );
            let data = ctx.accounts.slot_hashes.try_borrow_data()?;
            require!(data.len() >= 48, BracketChainError::SlotHashesUnavailable);
            // L-1 hardening. The raw most-recent slot hash is influenceable by
            // (and predictable to) the block leader producing this slot. Instead
            // of copying it verbatim, derive the seed by hashing several recent
            // SlotHashes entries together — no single leader controls the older
            // ones — bound to a domain tag and the tournament identity so the
            // fallback seed is unique per tournament even within one slot.
            //
            // SlotHashes layout: [u64 len][ (u64 slot, [u8;32] hash) ; len ],
            // most-recent entry first; header + one 40-byte entry = 48 bytes.
            const SEED_DOMAIN: &[u8] = b"bracketchain:seed:slot-hash:v1";
            const ENTRY_LEN: usize = 40;
            const MAX_MIX_ENTRIES: usize = 8;
            let entry_count = u64::from_le_bytes(
                data[0..8]
                    .try_into()
                    .map_err(|_| error!(BracketChainError::SlotHashesUnavailable))?,
            ) as usize;
            let mut hasher = keccak::Hasher::default();
            hasher.hash(SEED_DOMAIN);
            hasher.hash(tournament_key.as_ref());
            hasher.hash(ctx.accounts.tournament.organizer.as_ref());
            hasher.hash(&ctx.accounts.tournament.participant_count.to_le_bytes());
            // Mix at least one and up to MAX_MIX_ENTRIES recent slot entries
            // (slot number + hash), guarding against a truncated sysvar tail.
            for i in 0..entry_count.min(MAX_MIX_ENTRIES).max(1) {
                let off = 8 + i * ENTRY_LEN;
                if off + ENTRY_LEN > data.len() {
                    break;
                }
                hasher.hash(&data[off..off + ENTRY_LEN]);
            }
            let seed = hasher.result().to_bytes();
            drop(data);
            ctx.accounts.tournament.seed_hash = seed;
        }

        let pc = ctx.accounts.tournament.participant_count;
        let bracket_size = if pc.is_power_of_two() {
            pc
        } else {
            pc.checked_next_power_of_two()
                .ok_or(BracketChainError::ArithmeticOverflow)?
        };

        let tournament = &mut ctx.accounts.tournament;
        tournament.bracket_size = bracket_size;
        tournament.total_matches = bracket_size
            .checked_sub(1)
            .ok_or(BracketChainError::ArithmeticOverflow)?;
        tournament.status = TournamentStatus::PendingBracketInit;
    }

    require!(
        ctx.accounts.tournament.status == TournamentStatus::PendingBracketInit,
        BracketChainError::NotInRegistration
    );

    let bracket_size = ctx.accounts.tournament.bracket_size;
    let max_round = bracket_size.trailing_zeros() as u8;
    let space = 8 + MatchNode::INIT_SPACE;
    let lamports = Rent::get()?.minimum_balance(space);

    // H-2: non-OrganizerOnly tournaments must match the seed-derived permutation.
    // Re-derive it (cheap, deterministic) and validate each placed player against
    // it, consuming Participant accounts from the remaining-accounts tail.
    let vrf_mode = ctx.accounts.tournament.settlement_mode != SettlementMode::OrganizerOnly;
    let participant_count = ctx.accounts.tournament.participant_count;
    let perm: Vec<u16> = if vrf_mode {
        seed_permutation(&ctx.accounts.tournament.seed_hash, participant_count)
    } else {
        Vec::new()
    };
    let match_pdas = &ctx.remaining_accounts[..descriptors.len()];
    let participant_tail = &ctx.remaining_accounts[descriptors.len()..];
    let mut p_cursor: usize = 0;

    let mut byes_initialized: u16 = 0;

    for (descriptor, match_account) in descriptors.iter().zip(match_pdas.iter()) {
        require!(
            descriptor.round < max_round,
            BracketChainError::InvalidMatchIndex
        );
        let matches_in_round = bracket_size >> (descriptor.round + 1);
        require!(
            descriptor.match_index < matches_in_round,
            BracketChainError::InvalidMatchIndex
        );

        // Seed-consistency + membership (non-OrganizerOnly only). Consumes the
        // Participant accounts that prove `player == perm[rank]`.
        if vrf_mode {
            validate_descriptor_against_seed(
                descriptor,
                &perm,
                bracket_size,
                participant_count,
                tournament_key,
                ctx.program_id,
                participant_tail,
                &mut p_cursor,
            )?;
        }

        let bracket_arr = [descriptor.bracket];
        let round_arr = [descriptor.round];
        let match_index_le = descriptor.match_index.to_le_bytes();
        let bump_arr = [descriptor.bump];
        let signer_seeds: &[&[u8]] = &[
            MATCH_SEED,
            tournament_key.as_ref(),
            &bracket_arr,
            &round_arr,
            &match_index_le,
            &bump_arr,
        ];

        let expected_pda = Pubkey::create_program_address(signer_seeds, ctx.program_id)
            .map_err(|_| error!(BracketChainError::InvalidMatchIndex))?;
        require_keys_eq!(
            match_account.key(),
            expected_pda,
            BracketChainError::InvalidMatchIndex
        );

        system_program::create_account(
            CpiContext::new_with_signer(
                ctx.accounts.system_program.to_account_info(),
                system_program::CreateAccount {
                    from: ctx.accounts.organizer.to_account_info(),
                    to: match_account.clone(),
                },
                &[signer_seeds],
            ),
            lamports,
            space as u64,
            ctx.program_id,
        )?;

        let status = if descriptor.bye {
            MatchStatus::Completed
        } else if descriptor.player_a != Pubkey::default()
            && descriptor.player_b != Pubkey::default()
        {
            MatchStatus::Active
        } else {
            MatchStatus::Pending
        };

        let match_data = MatchNode {
            tournament: tournament_key,
            bracket: descriptor.bracket,
            round: descriptor.round,
            match_index: descriptor.match_index,
            player_a: descriptor.player_a,
            player_b: if descriptor.bye {
                Pubkey::default()
            } else {
                descriptor.player_b
            },
            winner: if descriptor.bye {
                descriptor.player_a
            } else {
                Pubkey::default()
            },
            status,
            bye: descriptor.bye,
            bump: descriptor.bump,
            // Envelope starts empty; only the player-reported / Oracle flow
            // ever populates it (OrganizerOnly uses `report_result` directly).
            proposal_source: crate::state::ProposalSource::None,
            proposer: Pubkey::default(),
            proposed_winner: Pubkey::default(),
            proposed_at: 0,
            claim_deadline: 0,
            disputed: false,
            dispute_reason: 0,
            // V1.2 Oracle commitment is bound later (`commit_match_lobby` /
            // `bind_match_feed`); unset for non-Oracle tournaments.
            commitment: None,
            switchboard_feed: Pubkey::default(),
        };

        let mut data = match_account.try_borrow_mut_data()?;
        let dst: &mut [u8] = &mut data;
        let mut writer: &mut [u8] = dst;
        match_data.try_serialize(&mut writer)?;

        if descriptor.bye {
            byes_initialized = byes_initialized
                .checked_add(1)
                .ok_or(BracketChainError::ArithmeticOverflow)?;
        }
    }

    // Every supplied Participant account must have been consumed — guards
    // against padding the tail with extra/unrelated accounts.
    if vrf_mode {
        require!(
            p_cursor == participant_tail.len(),
            BracketChainError::RemainingAccountsMismatch
        );
    }

    let descriptor_count = descriptors.len() as u16;
    let tournament = &mut ctx.accounts.tournament;
    tournament.matches_initialized = tournament
        .matches_initialized
        .checked_add(descriptor_count)
        .ok_or(BracketChainError::ArithmeticOverflow)?;
    tournament.matches_reported = tournament
        .matches_reported
        .checked_add(byes_initialized)
        .ok_or(BracketChainError::ArithmeticOverflow)?;

    if tournament.matches_initialized == tournament.total_matches {
        tournament.status = TournamentStatus::Active;
        tournament.started_at = Clock::get()?.unix_timestamp;

        emit!(TournamentStarted {
            event_version: EVENT_VERSION_V1,
            tournament: tournament_key,
            bracket_size: tournament.bracket_size,
            participant_count: tournament.participant_count,
            seed_hash: tournament.seed_hash,
            started_at: tournament.started_at,
        });
    }

    Ok(())
}

// ── H-2: seed-consistency + membership validation ───────────────────────────
//
// Validates one descriptor against the VRF-derived permutation `perm`, consuming
// the Participant accounts that prove the placed players are the seed-mandated
// ones. Round 0 carries both players (or a canonical bye); round 1 carries only
// the bye-propagated winners; round ≥ 2 must be empty (filled by `advance_winner`).
#[allow(clippy::too_many_arguments)]
fn validate_descriptor_against_seed<'info>(
    descriptor: &MatchInitDescriptor,
    perm: &[u16],
    bracket_size: u16,
    n: u16,
    tournament_key: Pubkey,
    program_id: &Pubkey,
    participant_tail: &[AccountInfo<'info>],
    cursor: &mut usize,
) -> Result<()> {
    let default = Pubkey::default();

    if descriptor.round == 0 {
        let (exp_a, exp_b) = round0_expected(perm, descriptor.match_index, bracket_size, n);
        // player_a (seed-rank == match_index) is always a real participant.
        consume_and_check(
            participant_tail,
            cursor,
            program_id,
            tournament_key,
            descriptor.player_a,
            exp_a,
        )?;
        match exp_b {
            Some(exp_b) => {
                require!(!descriptor.bye, BracketChainError::BracketSeedMismatch);
                require!(descriptor.player_b != default, BracketChainError::BracketSeedMismatch);
                consume_and_check(
                    participant_tail,
                    cursor,
                    program_id,
                    tournament_key,
                    descriptor.player_b,
                    exp_b,
                )?;
            }
            None => {
                // Canonical bye: the top seed advances alone.
                require!(descriptor.bye, BracketChainError::BracketSeedMismatch);
                require!(descriptor.player_b == default, BracketChainError::BracketSeedMismatch);
            }
        }
        return Ok(());
    }

    // Byes only originate in round 0; higher rounds are never byes.
    require!(!descriptor.bye, BracketChainError::BracketSeedMismatch);

    if descriptor.round == 1 {
        // Each slot is either fed by a round-0 bye (pre-filled with the bye
        // winner) or by a real match (default, filled later by advance_winner).
        let parent_a = (descriptor.match_index as usize) * 2;
        let parent_b = parent_a + 1;
        validate_round1_slot(
            descriptor.player_a,
            parent_a as u16,
            perm,
            bracket_size,
            n,
            participant_tail,
            cursor,
            program_id,
            tournament_key,
        )?;
        validate_round1_slot(
            descriptor.player_b,
            parent_b as u16,
            perm,
            bracket_size,
            n,
            participant_tail,
            cursor,
            program_id,
            tournament_key,
        )?;
    } else {
        // Round ≥ 2: no byes reach here; both slots fill via advance_winner.
        require!(descriptor.player_a == default, BracketChainError::BracketSeedMismatch);
        require!(descriptor.player_b == default, BracketChainError::BracketSeedMismatch);
    }

    Ok(())
}

/// Validates a round-1 slot: if its round-0 parent (`parent_match`) is a bye, the
/// slot must be pre-filled with that bye's winner (seed-rank `parent_match`);
/// otherwise the slot must be empty (the real parent's winner advances later).
#[allow(clippy::too_many_arguments)]
fn validate_round1_slot<'info>(
    slot_player: Pubkey,
    parent_match: u16,
    perm: &[u16],
    bracket_size: u16,
    n: u16,
    participant_tail: &[AccountInfo<'info>],
    cursor: &mut usize,
    program_id: &Pubkey,
    tournament_key: Pubkey,
) -> Result<()> {
    let (parent_winner_seed, parent_opponent) = round0_expected(perm, parent_match, bracket_size, n);
    if parent_opponent.is_none() {
        // Parent was a bye → its winner pre-fills this slot.
        require!(slot_player != Pubkey::default(), BracketChainError::BracketSeedMismatch);
        consume_and_check(
            participant_tail,
            cursor,
            program_id,
            tournament_key,
            slot_player,
            parent_winner_seed,
        )?;
    } else {
        require!(slot_player == Pubkey::default(), BracketChainError::BracketSeedMismatch);
    }
    Ok(())
}

/// Consumes the next Participant account from the tail and asserts it is a
/// genuine `Participant` of this tournament whose wallet == `expected_wallet`
/// and whose `seed_index == expected_seed_index` (the seed-mandated placement).
fn consume_and_check<'info>(
    tail: &[AccountInfo<'info>],
    cursor: &mut usize,
    program_id: &Pubkey,
    tournament_key: Pubkey,
    expected_wallet: Pubkey,
    expected_seed_index: u16,
) -> Result<()> {
    require!(*cursor < tail.len(), BracketChainError::RemainingAccountsMismatch);
    let ai = &tail[*cursor];
    *cursor += 1;

    require_keys_eq!(*ai.owner, *program_id, BracketChainError::NonParticipantInBracket);
    let participant: Participant = {
        let data = ai.try_borrow_data()?;
        Participant::try_deserialize(&mut &data[..])?
    };
    require_keys_eq!(
        participant.tournament,
        tournament_key,
        BracketChainError::NonParticipantInBracket
    );
    require_keys_eq!(
        participant.wallet,
        expected_wallet,
        BracketChainError::NonParticipantInBracket
    );
    require_eq!(
        participant.seed_index,
        expected_seed_index,
        BracketChainError::BracketSeedMismatch
    );
    Ok(())
}
