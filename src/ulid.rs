//! Sortable unique ids (ULIDs) for things that must stay the same across a
//! restart, a move and a handoff: a pane's key now, a task write's id later.
//!
//! 26 Crockford base32 characters: 48 bits of unix milliseconds, then 80
//! random bits. The random bits come from the standard library's per-process
//! hasher seeds, which the OS fills, so no extra dependency is needed.

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// A new id. Two calls never return the same value in one process.
pub(crate) fn new_ulid() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0);
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
    let random = random_80_bits(count);
    encode(millis, random)
}

/// Whether `value` has the shape of an id made by [`new_ulid`].
#[cfg(test)]
pub(crate) fn is_ulid(value: &str) -> bool {
    value.len() == 26
        && value
            .bytes()
            .all(|byte| ALPHABET.contains(&byte.to_ascii_uppercase()))
}

fn random_80_bits(count: u64) -> u128 {
    // `RandomState::new` draws fresh OS-seeded keys, so each hasher is
    // independent; the counter keeps two calls in one millisecond apart even
    // if the OS handed out equal keys.
    let mut high = RandomState::new().build_hasher();
    high.write_u64(count);
    let mut low = RandomState::new().build_hasher();
    low.write_u64(count);
    (u128::from(high.finish()) << 64 | u128::from(low.finish())) & ((1u128 << 80) - 1)
}

fn encode(millis: u64, random: u128) -> String {
    let value = (u128::from(millis & ((1 << 48) - 1)) << 80) | random;
    let mut out = [0u8; 26];
    for (index, slot) in out.iter_mut().enumerate() {
        let shift = 5 * (25 - index);
        *slot = ALPHABET[((value >> shift) & 0x1f) as usize];
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ulid_has_26_crockford_characters() {
        let id = new_ulid();
        assert_eq!(id.len(), 26);
        assert!(is_ulid(&id), "{id}");
        assert!(!is_ulid("short"));
        assert!(!is_ulid(&"U".repeat(26)), "U is not in the alphabet");
    }

    #[test]
    fn ulids_are_unique_and_the_timestamp_part_sorts() {
        let ids: std::collections::HashSet<String> = (0..1000).map(|_| new_ulid()).collect();
        assert_eq!(ids.len(), 1000);
        assert!(encode(1, 0) < encode(2, 0));
        assert_eq!(encode(0, 0), "0".repeat(26));
    }
}
