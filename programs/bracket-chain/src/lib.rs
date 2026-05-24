use anchor_lang::prelude::*;

pub mod constants;
pub mod errors;
pub mod events;
pub mod instructions;
pub mod state;

use instructions::*;
use state::{PayoutPreset, SettlementMode, SupportedGame};

declare_id!("AuXJKpuZtkegs2ZSgopgckhN7Ev8bUz4zBc238LD2F1");

#[program]
pub mod bracket_chain {
    use super::*;

    pub fn initialize_protocol(ctx: Context<InitializeProtocol>) -> Result<()> {
        instructions::initialize_protocol::handler(ctx)
    }

    pub fn create_tournament(
        ctx: Context<CreateTournament>,
        name: String,
        entry_fee: u64,
        max_participants: u16,
        payout_preset: PayoutPreset,
        registration_deadline: i64,
        organizer_deposit: u64,
        game: SupportedGame,
        settlement_mode: SettlementMode,
        dispute_window_secs: u32,
    ) -> Result<()> {
        instructions::create_tournament::handler(
            ctx,
            name,
            entry_fee,
            max_participants,
            payout_preset,
            registration_deadline,
            organizer_deposit,
            game,
            settlement_mode,
            dispute_window_secs,
        )
    }

    pub fn set_sas_config(
        ctx: Context<SetSasConfig>,
        sas_credential: Pubkey,
        sas_schemas: [Pubkey; 5],
    ) -> Result<()> {
        instructions::set_sas_config::handler(ctx, sas_credential, sas_schemas)
    }

    pub fn join_tournament(ctx: Context<JoinTournament>) -> Result<()> {
        instructions::join_tournament::handler(ctx)
    }

    pub fn start_tournament<'info>(
        ctx: Context<'_, '_, '_, 'info, StartTournament<'info>>,
        descriptors: Vec<MatchInitDescriptor>,
    ) -> Result<()> {
        instructions::start_tournament::handler(ctx, descriptors)
    }

    pub fn report_result<'info>(
        ctx: Context<'_, '_, '_, 'info, ReportResult<'info>>,
        winner: Pubkey,
        placements: Vec<Pubkey>,
    ) -> Result<()> {
        instructions::report_result::handler(ctx, winner, placements)
    }

    pub fn cancel_tournament<'info>(
        ctx: Context<'_, '_, '_, 'info, CancelTournament<'info>>,
    ) -> Result<()> {
        instructions::cancel_tournament::handler(ctx)
    }
}
