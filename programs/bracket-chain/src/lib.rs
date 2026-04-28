use anchor_lang::prelude::*;

pub mod constants;
pub mod errors;
pub mod events;
pub mod instructions;
pub mod state;

declare_id!("EF19YVUerm5QW1CsZeqiPDAFFtaXgdt6WuYBGeiz9Q1z");

#[program]
pub mod bracket_chain {
    use super::*;

    pub fn initialize(_ctx: Context<Initialize>) -> Result<()> {
        Ok(())
    }
}

#[derive(Accounts)]
pub struct Initialize {}
