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

    /// Devnet upgrade-only: grow a pre-V1 Tournament account to the V1 layout.
    /// Registration-phase tournaments only (see instruction docs).
    pub fn migrate_v1_tournament(ctx: Context<MigrateV1Tournament>) -> Result<()> {
        instructions::migrate_v1_tournament::handler(ctx)
    }

    // ── Verifiable bracket seeding (Switchboard On-Demand VRF, Stage B) ────

    /// Bind a committed Switchboard randomness account to the tournament.
    /// Opt-in; once bound, `start_tournament` requires the seed to be revealed.
    pub fn request_seed(ctx: Context<RequestSeed>) -> Result<()> {
        instructions::request_seed::handler(ctx)
    }

    /// Permissionless: consume the revealed randomness as the bracket seed.
    /// Must be bundled with Switchboard's reveal in the same transaction.
    pub fn reveal_seed(ctx: Context<RevealSeed>) -> Result<()> {
        instructions::reveal_seed::handler(ctx)
    }

    // ── Player-reported / Oracle settlement (Stage B) ──────────────────────

    /// A match player records the result they claim, opening the dispute
    /// window. Non-`OrganizerOnly` tournaments only.
    pub fn propose_result(ctx: Context<ProposeResult>, proposed_winner: Pubkey) -> Result<()> {
        instructions::propose_result::handler(ctx, proposed_winner)
    }

    /// The counterparty accepts the proposal, finalizing the match. Supply
    /// `placements` + payout ATAs (remaining_accounts) when it is the final.
    pub fn confirm_result<'info>(
        ctx: Context<'_, '_, '_, 'info, ConfirmResult<'info>>,
        placements: Vec<Pubkey>,
    ) -> Result<()> {
        instructions::confirm_result::handler(ctx, placements)
    }

    /// The counterparty rejects the proposal, escalating to the organizer.
    pub fn dispute_result(ctx: Context<DisputeResult>, dispute_reason: u8) -> Result<()> {
        instructions::dispute_result::handler(ctx, dispute_reason)
    }

    /// Permissionless: finalize an undisputed proposal after its dispute
    /// window. Supply `placements` + payout ATAs when it is the final match.
    pub fn claim_result<'info>(
        ctx: Context<'_, '_, '_, 'info, PermissionlessFinalize<'info>>,
        placements: Vec<Pubkey>,
    ) -> Result<()> {
        instructions::claim_result::handler(ctx, placements)
    }

    /// The organizer (arbitrator) settles a disputed match.
    pub fn resolve_dispute<'info>(
        ctx: Context<'_, '_, '_, 'info, ResolveDispute<'info>>,
        winner: Pubkey,
        placements: Vec<Pubkey>,
    ) -> Result<()> {
        instructions::resolve_dispute::handler(ctx, winner, placements)
    }

    /// Permissionless backstop: finalize a disputed match the organizer never
    /// resolved, 24h after the dispute.
    pub fn force_claim_disputed<'info>(
        ctx: Context<'_, '_, '_, 'info, PermissionlessFinalize<'info>>,
        placements: Vec<Pubkey>,
    ) -> Result<()> {
        instructions::force_claim_disputed::handler(ctx, placements)
    }
}
