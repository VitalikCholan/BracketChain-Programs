use anchor_lang::prelude::*;

#[account]
#[derive(InitSpace)]
pub struct Participant {
    pub tournament: Pubkey,
    pub wallet: Pubkey,
    pub seed_index: u16,
    pub refund_paid: bool,
    pub bump: u8,
    // ── V1.1 additions (appended — positional Borsh; never reorder above) ──
    /// keccak hash of the SAS attestation's `identity_bytes`. Zero-bytes for
    /// `Manual`-game tournaments (no attestation required).
    pub identity_hash: [u8; 32],
    /// The SAS Attestation account that bound this wallet ↔ game identity.
    /// `Pubkey::default()` for Manual tournaments.
    pub identity_attestation: Pubkey,
    /// Foundation stats consumed by partial-cancel (make-whole survivors),
    /// formats standings, and the webapp profile. Set by result-reporting ix.
    pub wins: u8,
    pub losses: u8,
    pub points_for: u32,
    pub points_against: u32,
}
