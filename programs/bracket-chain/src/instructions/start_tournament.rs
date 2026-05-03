use anchor_lang::prelude::*;
use anchor_lang::solana_program::keccak;
use anchor_lang::solana_program::sysvar::slot_hashes;
use anchor_lang::system_program;

use crate::constants::{MATCH_SEED, MIN_PARTICIPANTS};
use crate::errors::BracketChainError;
use crate::events::TournamentStarted;
use crate::state::{MatchNode, MatchStatus, Tournament, TournamentStatus};

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct MatchInitDescriptor {
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
            &keccak::hashv(&[tournament.name.as_bytes()]).0,
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
    require!(
        descriptors.len() == ctx.remaining_accounts.len(),
        BracketChainError::RemainingAccountsMismatch
    );

    let tournament_key = ctx.accounts.tournament.key();

    // First chunk: capture seed_hash + compute bracket dimensions.
    if ctx.accounts.tournament.status == TournamentStatus::Registration {
        require!(
            ctx.accounts.tournament.participant_count >= MIN_PARTICIPANTS,
            BracketChainError::MinParticipantsNotMet
        );

        let data = ctx.accounts.slot_hashes.try_borrow_data()?;
        require!(data.len() >= 48, BracketChainError::SlotHashesUnavailable);
        let mut seed = [0u8; 32];
        seed.copy_from_slice(&data[16..48]);
        drop(data);

        let pc = ctx.accounts.tournament.participant_count;
        let bracket_size = if pc.is_power_of_two() {
            pc
        } else {
            pc.checked_next_power_of_two()
                .ok_or(BracketChainError::ArithmeticOverflow)?
        };

        let tournament = &mut ctx.accounts.tournament;
        tournament.seed_hash = seed;
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

    let mut byes_initialized: u16 = 0;

    for (descriptor, match_account) in descriptors.iter().zip(ctx.remaining_accounts.iter()) {
        require!(
            descriptor.round < max_round,
            BracketChainError::InvalidMatchIndex
        );
        let matches_in_round = bracket_size >> (descriptor.round + 1);
        require!(
            descriptor.match_index < matches_in_round,
            BracketChainError::InvalidMatchIndex
        );

        let round_arr = [descriptor.round];
        let match_index_le = descriptor.match_index.to_le_bytes();
        let bump_arr = [descriptor.bump];
        let signer_seeds: &[&[u8]] = &[
            MATCH_SEED,
            tournament_key.as_ref(),
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
            tournament: tournament_key,
            bracket_size: tournament.bracket_size,
            participant_count: tournament.participant_count,
            seed_hash: tournament.seed_hash,
            started_at: tournament.started_at,
        });
    }

    Ok(())
}
