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
}
