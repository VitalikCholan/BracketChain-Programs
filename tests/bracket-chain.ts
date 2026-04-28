import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { BracketChain } from "../target/types/bracket_chain";

describe("bracket-chain", () => {
  anchor.setProvider(anchor.AnchorProvider.env());

  const program = anchor.workspace.bracketChain as Program<BracketChain>;

  it("Is initialized!", async () => {
    const tx = await program.methods.initialize().rpc();
    console.log("Your transaction signature", tx);
  });
});
