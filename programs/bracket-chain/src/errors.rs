use anchor_lang::prelude::*;

#[error_code]
pub enum BracketChainError {
    #[msg("Caller is not the authorized authority for this action")]
    UnauthorizedAuthority,

    #[msg("Tournament has reached its maximum participant count")]
    TournamentFull,

    #[msg("Wallet is already registered for this tournament")]
    AlreadyRegistered,

    #[msg("Registration window for this tournament is closed")]
    RegistrationClosed,

    #[msg("Tournament is not in the Registration state")]
    NotInRegistration,

    #[msg("Tournament is not in the Active state")]
    NotActive,

    #[msg("Tournament is not in the Completed state")]
    NotCompleted,

    #[msg("Selected payout preset is invalid")]
    InvalidPayoutPreset,

    #[msg("Selected payout preset requires more participants than configured")]
    PresetExceedsParticipants,

    #[msg("Match has already been reported")]
    MatchAlreadyReported,

    #[msg("Reported winner is not a participant of the tournament")]
    NonParticipantWinner,

    #[msg("Cannot cancel a tournament that has matches in progress")]
    TournamentInProgress,

    #[msg("Refund has already been issued to this participant")]
    RefundAlreadyIssued,

    #[msg("Participant count exceeds the protocol maximum (128)")]
    MaxParticipantsExceeded,

    #[msg("Participant count is below the protocol minimum (2)")]
    MinParticipantsNotMet,

    #[msg("Tournament name exceeds 32 bytes")]
    NameTooLong,

    #[msg("Provided token mint is invalid for this tournament")]
    InvalidTokenMint,

    #[msg("Provided vault token account does not match the tournament vault")]
    InvalidVault,

    #[msg("Provided treasury token account does not match the protocol treasury")]
    InvalidTreasury,

    #[msg("Match referenced is outside the bracket")]
    InvalidMatchIndex,

    #[msg("Match parents not yet completed; cannot report this match")]
    ParentMatchesNotComplete,

    #[msg("remaining_accounts does not match expected count for this instruction")]
    RemainingAccountsMismatch,

    #[msg("Arithmetic overflow")]
    ArithmeticOverflow,

    #[msg("slot_hashes sysvar is empty; cannot derive seed")]
    SlotHashesUnavailable,

    #[msg("Selected game is not yet supported for tournament creation")]
    GameNotYetSupported,

    #[msg("This game requires a SAS identity attestation to join")]
    AttestationRequired,

    #[msg("Attestation account is not owned by the SAS program")]
    InvalidAttestationOwner,

    #[msg("Attestation credential does not match the protocol's SAS credential")]
    WrongAttestationCredential,

    #[msg("Attestation schema does not match the game's SAS schema")]
    WrongAttestationSchema,

    #[msg("Attestation nonce does not bind to the joining wallet")]
    AttestationWalletMismatch,

    #[msg("Attestation has expired")]
    AttestationExpired,

    #[msg("Attestation account data is malformed")]
    MalformedAttestation,

    // ── Player-reported / Oracle settlement (Stage B) ──────────────────────
    #[msg("This action is not allowed for the tournament's settlement mode")]
    SettlementModeMismatch,

    #[msg("Signer is not a player in this match")]
    NotPlayerInMatch,

    #[msg("Only the counterparty may confirm or dispute this proposal")]
    NotCounterparty,

    #[msg("Match has no pending proposal")]
    NoProposal,

    #[msg("Match already has a pending proposal")]
    ProposalAlreadyExists,

    #[msg("Proposed winner is not a player in this match")]
    InvalidProposedWinner,

    #[msg("Claim window has not elapsed yet")]
    ClaimWindowNotElapsed,

    #[msg("Proposal is disputed; it cannot be claimed")]
    ProposalDisputed,

    #[msg("Proposal is not disputed")]
    ProposalNotDisputed,

    #[msg("Tournament seed has not been revealed; start is gated on VRF")]
    SeedNotRevealed,

    #[msg("Switchboard randomness is not yet resolved for this slot")]
    RandomnessNotResolved,

    #[msg("Provided randomness account does not match the tournament commitment")]
    RandomnessAccountMismatch,

    #[msg("Randomness account is not owned by the Switchboard On-Demand program")]
    InvalidRandomnessOwner,

    #[msg("Randomness account data is malformed")]
    MalformedRandomness,

    #[msg("Tournament seed has already been revealed")]
    SeedAlreadyRevealed,

    #[msg("Account is not a Tournament owned by this program")]
    InvalidTournamentAccount,

    #[msg("Tournament account is already at the V1 layout; migration not needed")]
    MigrationNotNeeded,

    // ── V1.2 Oracle settlement (Stage C) ───────────────────────────────────
    #[msg("Match already has a lobby commitment")]
    MatchAlreadyCommitted,

    #[msg("Match has no lobby commitment; commit before binding a feed")]
    MatchNotCommitted,

    #[msg("Switchboard feed account is not owned by the On-Demand program, or is on the wrong queue")]
    WrongFeedAccount,

    #[msg("Oracle feed value did not match either committed player identity")]
    OracleWinnerNotInMatch,

    #[msg("Signer is not authorized to dispute this Oracle proposal")]
    NotAuthorized,

    #[msg("Proposal source is not valid for this action")]
    BadProposalSource,

    // ── Stage D (program improvements; appended — never reorder above) ───────
    #[msg("Custom payout split is invalid (bps must sum to 10000, be gapless, and fund the winner)")]
    InvalidCustomPayout,

    // ── H-1 hardening (appended — never reorder above) ──────────────────────
    #[msg("A multi-placement (non-WinnerTakesAll) final may only be finalized by a trusted signer (settle_final / report_result / resolve_dispute), not a permissionless or counterparty path")]
    UntrustedMultiPlacementFinal,

    // ── H-2 hardening (appended — never reorder above) ──────────────────────
    #[msg("Bracket descriptor is inconsistent with the VRF-derived seed permutation")]
    BracketSeedMismatch,

    #[msg("Account supplied for a bracket slot is not a Participant of this tournament")]
    NonParticipantInBracket,
}
