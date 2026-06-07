use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer};

use crate::constants::{
    EVENT_VERSION_V1, PARTICIPANT_SEED, PROTOCOL_CONFIG_SEED, SAS_PROGRAM_ID, VAULT_SEED,
};
use crate::errors::BracketChainError;
use crate::events::ParticipantRegistered;
use crate::state::{Participant, ProtocolConfig, SupportedGame, Tournament, TournamentStatus};

#[derive(Accounts)]
pub struct JoinTournament<'info> {
    #[account(mut)]
    pub player: Signer<'info>,

    #[account(
        mut,
        seeds = [
            crate::constants::TOURNAMENT_SEED,
            tournament.organizer.as_ref(),
            tournament.name.as_bytes(),
        ],
        bump = tournament.bump,
    )]
    // Boxed: Tournament grew in V1.1; with protocol_config added, an unboxed
    // pair would overflow the SBF 4KB stack frame in try_accounts.
    pub tournament: Box<Account<'info, Tournament>>,

    #[account(
        seeds = [PROTOCOL_CONFIG_SEED],
        bump = protocol_config.bump,
    )]
    pub protocol_config: Box<Account<'info, ProtocolConfig>>,

    #[account(
        init,
        payer = player,
        space = 8 + Participant::INIT_SPACE,
        seeds = [PARTICIPANT_SEED, tournament.key().as_ref(), player.key().as_ref()],
        bump,
    )]
    pub participant: Account<'info, Participant>,

    #[account(
        mut,
        constraint = player_token_account.mint == tournament.token_mint
            @ BracketChainError::InvalidTokenMint,
        constraint = player_token_account.owner == player.key()
            @ BracketChainError::UnauthorizedAuthority,
    )]
    pub player_token_account: Account<'info, TokenAccount>,

    #[account(
        mut,
        seeds = [VAULT_SEED, tournament.key().as_ref()],
        bump = tournament.vault_bump,
    )]
    pub vault: Account<'info, TokenAccount>,

    /// CHECK: hand-validated by `validate_attestation` (owner == SAS program,
    /// credential + schema match `protocol_config`, nonce == player, not
    /// expired). Required when `tournament.game != Manual`; pass `None` for
    /// Manual tournaments.
    pub game_identity_attestation: Option<UncheckedAccount<'info>>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

pub(crate) fn handler(ctx: Context<JoinTournament>) -> Result<()> {
    let now = Clock::get()?.unix_timestamp;
    let game = ctx.accounts.tournament.game;

    // V1.1 identity gate (before mutating state): non-Manual games require a
    // valid SAS attestation binding this wallet ↔ game identity. Manual skips it.
    let (identity_hash, identity_attestation) = if game == SupportedGame::Manual {
        ([0u8; 32], Pubkey::default())
    } else {
        let attestation = ctx
            .accounts
            .game_identity_attestation
            .as_ref()
            .ok_or(error!(BracketChainError::AttestationRequired))?;
        let credential = ctx.accounts.protocol_config.sas_credential;
        let schema = ctx.accounts.protocol_config.sas_schemas[game as usize];
        validate_attestation(
            &attestation.to_account_info(),
            &ctx.accounts.player.key(),
            &credential,
            &schema,
            now,
        )?
    };

    let tournament = &mut ctx.accounts.tournament;

    require!(
        tournament.status == TournamentStatus::Registration,
        BracketChainError::NotInRegistration
    );
    require!(
        now < tournament.registration_deadline,
        BracketChainError::RegistrationClosed
    );
    require!(
        tournament.participant_count < tournament.max_participants,
        BracketChainError::TournamentFull
    );

    let participant_index = tournament.participant_count;

    let cpi_accounts = Transfer {
        from: ctx.accounts.player_token_account.to_account_info(),
        to: ctx.accounts.vault.to_account_info(),
        authority: ctx.accounts.player.to_account_info(),
    };
    let cpi_ctx = CpiContext::new(ctx.accounts.token_program.to_account_info(), cpi_accounts);
    token::transfer(cpi_ctx, tournament.entry_fee)?;

    let participant = &mut ctx.accounts.participant;
    participant.tournament = tournament.key();
    participant.wallet = ctx.accounts.player.key();
    participant.seed_index = participant_index;
    participant.refund_paid = false;
    participant.bump = ctx.bumps.participant;
    participant.identity_hash = identity_hash;
    participant.identity_attestation = identity_attestation;

    tournament.participant_count = tournament
        .participant_count
        .checked_add(1)
        .ok_or(BracketChainError::ArithmeticOverflow)?;

    emit!(ParticipantRegistered {
        event_version: EVENT_VERSION_V1,
        tournament: tournament.key(),
        wallet: participant.wallet,
        participant_index,
    });

    Ok(())
}

/// Manually parse + validate a SAS Attestation account for the joining wallet.
///
/// SAS account layout (sas-lib 1.0.x): `u8 discriminator | nonce(32) |
/// credential(32) | schema(32) | data(u32 len + bytes) | signer(32) |
/// expiry(i64) | tokenAccount(32)`. Our schema's `data` is borsh
/// `{ game: u8, steam_id_64: u64, identity_bytes: Vec<u8> }` — SAS has no
/// fixed-size array type, so `identity_bytes` is length-prefixed.
///
/// `identity_bytes` is the 32-byte identity fingerprint the issuer (indexer)
/// derives off-chain (e.g. a hash of the Steam ID); the program stores it
/// verbatim as `participant.identity_hash` — no on-chain hashing. Returns
/// `(identity_hash, attestation_pubkey)`.
fn validate_attestation(
    att_ai: &AccountInfo,
    player: &Pubkey,
    expected_credential: &Pubkey,
    expected_schema: &Pubkey,
    now: i64,
) -> Result<([u8; 32], Pubkey)> {
    require_keys_eq!(
        *att_ai.owner,
        SAS_PROGRAM_ID,
        BracketChainError::InvalidAttestationOwner
    );

    let data = att_ai.try_borrow_data()?;
    // Fixed header up to the start of the variable `data` field: 1 + 32*3 + 4.
    require!(data.len() >= 101, BracketChainError::MalformedAttestation);

    let nonce = Pubkey::try_from(&data[1..33])
        .map_err(|_| error!(BracketChainError::MalformedAttestation))?;
    let credential = Pubkey::try_from(&data[33..65])
        .map_err(|_| error!(BracketChainError::MalformedAttestation))?;
    let schema = Pubkey::try_from(&data[65..97])
        .map_err(|_| error!(BracketChainError::MalformedAttestation))?;

    require_keys_eq!(nonce, *player, BracketChainError::AttestationWalletMismatch);
    require_keys_eq!(
        credential,
        *expected_credential,
        BracketChainError::WrongAttestationCredential
    );
    require_keys_eq!(
        schema,
        *expected_schema,
        BracketChainError::WrongAttestationSchema
    );

    // `data` field: u32 LE length at 97..101, payload at 101..101+data_len.
    let data_len = u32::from_le_bytes(
        data[97..101]
            .try_into()
            .map_err(|_| error!(BracketChainError::MalformedAttestation))?,
    ) as usize;
    let payload_start = 101usize;
    let payload_end = payload_start
        .checked_add(data_len)
        .ok_or(error!(BracketChainError::MalformedAttestation))?;
    // After the payload: signer(32) + expiry(8) must fit.
    let expiry_start = payload_end
        .checked_add(32)
        .ok_or(error!(BracketChainError::MalformedAttestation))?;
    require!(
        data.len() >= expiry_start + 8,
        BracketChainError::MalformedAttestation
    );

    // Payload borsh: game(1) + steam_id_64(8) + identity_bytes(Vec<u8>).
    // The identity_bytes length prefix sits at payload offset 9.
    let payload = &data[payload_start..payload_end];
    require!(payload.len() >= 13, BracketChainError::MalformedAttestation);
    let ib_len = u32::from_le_bytes(
        payload[9..13]
            .try_into()
            .map_err(|_| error!(BracketChainError::MalformedAttestation))?,
    ) as usize;
    // identity_bytes must be the canonical 32-byte fingerprint.
    require!(ib_len == 32, BracketChainError::MalformedAttestation);
    let ib_start = 13usize;
    let ib_end = ib_start + 32;
    require!(
        payload.len() >= ib_end,
        BracketChainError::MalformedAttestation
    );

    // expiry: 0 means "never expires"; otherwise must be in the future.
    let expiry = i64::from_le_bytes(
        data[expiry_start..expiry_start + 8]
            .try_into()
            .map_err(|_| error!(BracketChainError::MalformedAttestation))?,
    );
    require!(
        expiry == 0 || now < expiry,
        BracketChainError::AttestationExpired
    );

    let mut identity_hash = [0u8; 32];
    identity_hash.copy_from_slice(&payload[ib_start..ib_end]);
    Ok((identity_hash, *att_ai.key))
}
