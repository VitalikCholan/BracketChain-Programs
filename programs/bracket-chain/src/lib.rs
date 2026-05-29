use anchor_lang::prelude::*;

pub mod constants;
pub mod errors;
pub mod events;
pub mod instructions;
pub mod state;

use instructions::*;
use state::{PayoutPreset, SettlementMode, SupportedGame};

declare_id!("3YpkUKBh8288XN2dCKSwBnEdyc5UozSJ19A1ZCLpUZsZ");

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

    /// Admin (Stage C / V1.2): set Switchboard On-Demand settlement params.
    pub fn set_oracle_config(
        ctx: Context<SetOracleConfig>,
        switchboard_queue: Pubkey,
        max_stale_slots: u32,
        min_oracle_samples: u32,
    ) -> Result<()> {
        instructions::set_oracle_config::handler(
            ctx,
            switchboard_queue,
            max_stale_slots,
            min_oracle_samples,
        )
    }

    /// Organizer (Stage C / V1.2): commit a match to a game lobby pre-launch.
    pub fn commit_match_lobby(
        ctx: Context<CommitMatchLobby>,
        lobby_id: [u8; 16],
        expected_feed_hash: [u8; 32],
    ) -> Result<()> {
        instructions::commit_match_lobby::handler(ctx, lobby_id, expected_feed_hash)
    }

    /// Organizer (Stage C / V1.2): bind a Switchboard PullFeed to a committed match.
    pub fn bind_match_feed(ctx: Context<BindMatchFeed>) -> Result<()> {
        instructions::bind_match_feed::handler(ctx)
    }

    /// Permissionless (Stage C / V1.2): write the oracle-reported winner into
    /// the proposal envelope (`source = Oracle`).
    pub fn propose_result_oracle(ctx: Context<ProposeResultOracle>) -> Result<()> {
        instructions::propose_result_oracle::handler(ctx)
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

    /// Permissionless rent reclaim for a terminal tournament (Stage D, D-3).
    /// Closes child PDAs passed in `remaining_accounts`; `close_root` closes the
    /// vault + Tournament PDA on the final call. All rent → organizer.
    pub fn close_tournament<'info>(
        ctx: Context<'_, '_, '_, 'info, CloseTournament<'info>>,
        close_root: bool,
    ) -> Result<()> {
        instructions::close_tournament::handler(ctx, close_root)
    }

    /// Organizer-signed mid-tournament cancellation (Stage E, E-2). Flips an
    /// `Active` tournament to `PartialCancelled`; refunds run via
    /// `partial_refund_chunk`.
    pub fn partial_cancel_tournament(
        ctx: Context<PartialCancelTournament>,
    ) -> Result<()> {
        instructions::partial_cancel_tournament::handler(ctx)
    }

    /// Permissionless full-refund processing for a partially-cancelled
    /// tournament (Stage E, E-3). Refunds every participant their full entry
    /// fee + returns the organizer deposit. Chunked via `remaining_accounts`.
    pub fn partial_refund_chunk<'info>(
        ctx: Context<'_, '_, '_, 'info, PartialRefundChunk<'info>>,
    ) -> Result<()> {
        instructions::partial_refund_chunk::handler(ctx)
    }

    /// Devnet upgrade-only: grow a pre-V1 Tournament account to the V1 layout.
    /// Registration-phase tournaments only (see instruction docs).
    pub fn migrate_v1_tournament(ctx: Context<MigrateV1Tournament>) -> Result<()> {
        instructions::migrate_v1_tournament::handler(ctx)
    }

    /// Admin one-shot: realloc `ProtocolConfig` from the pre-V1.1 layout
    /// (107 bytes) to the current `INIT_SPACE`. Required after an in-place
    /// upgrade across Stages B (SAS) / C (Oracle) when the existing PDA was
    /// initialized under V1.0 and never grown. New bytes are zero-filled; the
    /// authority follows up with `set_sas_config` / `set_oracle_config` to
    /// populate the new fields.
    pub fn migrate_protocol_config(ctx: Context<MigrateProtocolConfig>) -> Result<()> {
        instructions::migrate_protocol_config::handler(ctx)
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
