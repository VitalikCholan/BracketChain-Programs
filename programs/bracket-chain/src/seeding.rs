//! Canonical, cross-language-deterministic bracket seeding (H-2 fix).
//!
//! The bracket assignment is derived from `tournament.seed_hash` (VRF-revealed)
//! so it is **unmanipulable**: the organizer cannot choose who plays whom. The
//! SDK builds the `MatchInitDescriptor[]` from this exact algorithm client-side
//! and `start_tournament` re-derives the permutation to validate the
//! descriptors are consistent with the seed (see
//! `bracketchain-v1-player-reported-plan.md` §"Modify: start_tournament.rs").
//!
//! Both sides MUST produce byte-identical permutations. The PRNG is a
//! dependency-free `splitmix64` over wrapping `u64` arithmetic (no on-chain
//! hash syscall), so the TypeScript SDK reproduces it exactly with masked
//! `BigInt` ops. Single-elimination only (V1); `bracket: u8` lane stays `0`.

/// Folds the 32-byte VRF seed into a 64-bit `splitmix64` state. Absorbs each of
/// the four 8-byte little-endian words and runs the `splitmix64` finalizer
/// **after every word**, so every byte matters and symmetric inputs (e.g. equal
/// repeated words) do not cancel as a plain XOR-fold would. The SDK mirrors this
/// absorption loop exactly.
fn seed_state(seed_hash: &[u8; 32]) -> u64 {
    let mut s: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut k = 0;
    while k < 32 {
        let w = u64::from_le_bytes([
            seed_hash[k],
            seed_hash[k + 1],
            seed_hash[k + 2],
            seed_hash[k + 3],
            seed_hash[k + 4],
            seed_hash[k + 5],
            seed_hash[k + 6],
            seed_hash[k + 7],
        ]);
        s ^= w;
        s = (s ^ (s >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        s = (s ^ (s >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        s ^= s >> 31;
        k += 8;
    }
    s
}

/// `splitmix64`: advance `state`, return the next 64-bit value. Exact wrapping
/// arithmetic — the SDK mirrors this with `& 0xFFFF_FFFF_FFFF_FFFF` BigInt ops.
fn next_u64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Fisher-Yates permutation of `[0, n)` seeded by `seed_hash`. `perm[rank]` is
/// the **seed_index** (= join order) of the participant placed at bracket
/// seed-rank `rank`. Pure + cross-language deterministic. `n ≤ 128`.
///
/// Iterates `i` from `n-1` down to `1`, swapping `perm[i]` with `perm[j]` where
/// `j = next_u64() % (i + 1) ∈ [0, i]`. The SDK reproduces this loop verbatim.
pub fn seed_permutation(seed_hash: &[u8; 32], n: u16) -> Vec<u16> {
    let n = n as usize;
    let mut perm: Vec<u16> = (0..n as u16).collect();
    let mut state = seed_state(seed_hash);
    let mut i = n;
    while i > 1 {
        i -= 1;
        let j = (next_u64(&mut state) % (i as u64 + 1)) as usize;
        perm.swap(i, j);
    }
    perm
}

/// Expected round-0 assignment for match `m` under standard single-elim seeding:
/// seed-rank `m` vs seed-rank `bracket_size - 1 - m`. Returns the participants'
/// **seed_index** values: `(player_a_seed_index, Option<player_b_seed_index>)`
/// where `None` == bye.
///
/// `player_a` (rank `m`) is always a real participant: `m < bracket_size/2 < n`
/// because `bracket_size = next_pow2(n)` ⇒ `n > bracket_size/2`. The opponent
/// rank `bracket_size-1-m` is a bye exactly when it is `≥ n`, which places the
/// `bracket_size - n` byes against the **top** seeds, one per match — never two
/// byes in a match.
pub fn round0_expected(perm: &[u16], m: u16, bracket_size: u16, n: u16) -> (u16, Option<u16>) {
    let a_rank = m;
    let b_rank = bracket_size - 1 - m;
    let a = perm[a_rank as usize];
    let b = if b_rank < n {
        Some(perm[b_rank as usize])
    } else {
        None
    };
    (a, b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seed(byte: u8) -> [u8; 32] {
        [byte; 32]
    }

    fn is_permutation(perm: &[u16], n: u16) -> bool {
        if perm.len() != n as usize {
            return false;
        }
        let mut seen = vec![false; n as usize];
        for &p in perm {
            if p >= n || seen[p as usize] {
                return false;
            }
            seen[p as usize] = true;
        }
        true
    }

    #[test]
    fn cross_language_golden_vector() {
        // Shared with the SDK `seeding.test.ts` — pins byte-identical splitmix64
        // Fisher-Yates across Rust and TypeScript. Seed = bytes 0..31, n = 8.
        let mut s = [0u8; 32];
        let mut i = 0;
        while i < 32 {
            s[i] = i as u8;
            i += 1;
        }
        assert_eq!(seed_permutation(&s, 8), vec![4u16, 0, 3, 1, 5, 6, 7, 2]);
    }

    #[test]
    fn deterministic_same_seed_same_permutation() {
        let s = seed(7);
        assert_eq!(seed_permutation(&s, 64), seed_permutation(&s, 64));
    }

    #[test]
    fn different_seed_changes_order() {
        // Overwhelmingly likely to differ for n = 64; guards against a constant.
        assert_ne!(seed_permutation(&seed(1), 64), seed_permutation(&seed(2), 64));
    }

    #[test]
    fn always_a_bijection_over_participants() {
        for n in [2u16, 3, 5, 7, 8, 17, 64, 100, 128] {
            assert!(
                is_permutation(&seed_permutation(&seed(n as u8), n), n),
                "n={n} must yield a permutation of 0..n"
            );
        }
    }

    #[test]
    fn power_of_two_has_no_byes() {
        let (n, bracket) = (8u16, 8u16);
        let perm = seed_permutation(&seed(9), n);
        for m in 0..(bracket / 2) {
            assert!(round0_expected(&perm, m, bracket, n).1.is_some());
        }
    }

    #[test]
    fn bye_count_equals_bracket_minus_n_and_player_a_always_real() {
        // n=5 → bracket_size 8 → 3 byes, each alone in a match.
        let (n, bracket) = (5u16, 8u16);
        let perm = seed_permutation(&seed(3), n);
        let mut byes = 0;
        for m in 0..(bracket / 2) {
            let (a, b) = round0_expected(&perm, m, bracket, n);
            assert!(a < n, "player_a (rank m) is always a real participant");
            if b.is_none() {
                byes += 1;
            }
        }
        assert_eq!(byes, bracket - n, "exactly bracket_size - n byes");
    }

    #[test]
    fn round0_covers_every_participant_exactly_once() {
        for (n, bracket) in [(5u16, 8u16), (6, 8), (7, 8), (12, 16), (100, 128)] {
            let perm = seed_permutation(&seed(n as u8), n);
            let mut seen = vec![false; n as usize];
            let mut reals = 0u16;
            for m in 0..(bracket / 2) {
                let (a, b) = round0_expected(&perm, m, bracket, n);
                assert!(!seen[a as usize]);
                seen[a as usize] = true;
                reals += 1;
                if let Some(bv) = b {
                    assert!(!seen[bv as usize]);
                    seen[bv as usize] = true;
                    reals += 1;
                }
            }
            assert_eq!(reals, n, "every participant placed exactly once (n={n})");
            assert!(seen.iter().all(|&x| x));
        }
    }
}
