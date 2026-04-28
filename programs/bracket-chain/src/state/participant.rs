use anchor_lang::prelude::*;

#[account]
#[derive(InitSpace)]
pub struct Participant {
    pub tournament: Pubkey,
    pub wallet: Pubkey,
    pub seed_index: u16,
    pub refund_paid: bool,
    pub bump: u8,
}
