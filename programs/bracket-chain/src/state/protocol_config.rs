use anchor_lang::prelude::*;

#[account]
#[derive(InitSpace)]
pub struct ProtocolConfig {
    pub authority: Pubkey,
    pub treasury: Pubkey,
    /// Recommended default token mint (advisory only — clients may show this
    /// as the "default" / "preferred" mint in their UI). Per-tournament
    /// `tournament.token_mint` is NOT constrained against this — any SPL
    /// mint (USDC, wSOL, custom) can be used per tournament.
    pub default_mint: Pubkey,
    pub fee_bps: u16,
    pub bump: u8,
    // ── V1.1 additions (appended — positional Borsh; never reorder above) ──
    /// BracketChain's SAS Credential PDA (issuer = indexer's sas-issuer key).
    /// `join_tournament` validates an attestation's credential against this.
    pub sas_credential: Pubkey,
    /// One SAS Schema PDA per `SupportedGame` variant, indexed by discriminant
    /// (`sas_schemas[game as usize]`). `Manual` (index 0) is unused. Set via
    /// `set_sas_config`; unset slots are `Pubkey::default()`.
    pub sas_schemas: [Pubkey; 5],
}
