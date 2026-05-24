use anchor_lang::prelude::*;

/// Game a tournament is played in. **Discriminants are a stable wire contract** —
/// indexers and the SAS game-identity schema lookup (`protocol_config.sas_schemas`)
/// index on these values, so never reorder or renumber. `Manual` (0) needs no
/// identity attestation; every other variant requires a SAS attestation at
/// `join_tournament`. Phase 1 only accepts `Manual` + `Dota2` at create-time
/// (the rest are reserved so the field stays positionally fixed).
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, Debug, InitSpace)]
pub enum SupportedGame {
    Manual = 0,
    Dota2 = 1,
    Cs2Faceit = 2,
    Valorant = 3,
    LoL = 4,
}

/// Who may report match results for a tournament. Locked at create-time.
/// `OrganizerOnly` (0) is the MVP behaviour and the only mode with a live
/// reporting path today; `PlayerReported` + `Oracle` ship their reporting
/// instructions in later V1 stages of this same redeploy. The enum ships now so
/// `tournament.settlement_mode` is positionally stable across the redeploy.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, Debug, InitSpace)]
pub enum SettlementMode {
    OrganizerOnly = 0,
    PlayerReported = 1,
    Oracle = 2,
}
