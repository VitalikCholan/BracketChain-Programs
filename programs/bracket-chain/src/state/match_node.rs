use anchor_lang::prelude::*;

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, Debug, InitSpace)]
pub enum MatchStatus {
    Pending,
    Active,
    Completed,
}

#[account]
#[derive(InitSpace)]
pub struct MatchNode {
    pub tournament: Pubkey,
    pub round: u8,
    pub match_index: u16,
    pub player_a: Pubkey,
    pub player_b: Pubkey,
    pub winner: Pubkey,
    pub status: MatchStatus,
    pub bye: bool,
    pub bump: u8,
}
