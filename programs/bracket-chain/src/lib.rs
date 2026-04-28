use anchor_lang::prelude::*;

declare_id!("EF19YVUerm5QW1CsZeqiPDAFFtaXgdt6WuYBGeiz9Q1z");

#[program]
pub mod bracket_chain {
    use super::*;

    pub fn initialize(ctx: Context<Initialize>) -> Result<()> {
        msg!("Greetings from: {:?}", ctx.program_id);
        Ok(())
    }
}

#[derive(Accounts)]
pub struct Initialize {}
