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
}
