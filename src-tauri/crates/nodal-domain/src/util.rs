//! Id generation and validation, and small text helpers. Time, disk and blocking helpers
//! live behind ports (`Clock`, `PlanFiles`) or in the shell.

use std::sync::atomic::{AtomicU32, Ordering};

static SEQ: AtomicU32 = AtomicU32::new(0);

/// Time-sortable id, ULID-style with no dependencies: `prefix` + ms in base32 (10) +
/// process sequence (3) + 5 random characters (hash seeded by `RandomState`).
/// Only `[0-9a-z]`: it ends up in paths (`tasks/<id>/`).
pub fn new_id(prefix: char, now: i64) -> String {
    use std::hash::{BuildHasher, Hasher};
    debug_assert!(prefix.is_ascii_lowercase());
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_i64(now);
    h.write_u32(seq);
    h.write_u32(std::process::id());
    let rnd = h.finish();
    format!(
        "{prefix}{}{}{}",
        base32(now as u64, 10),
        base32(seq as u64, 3),
        base32(rnd, 5)
    )
}

fn base32(mut n: u64, width: usize) -> String {
    const ALPHABET: &[u8; 32] = b"0123456789abcdefghjkmnpqrstvwxyz";
    let mut out = vec![b'0'; width];
    for slot in out.iter_mut().rev() {
        *slot = ALPHABET[(n % 32) as usize];
        n /= 32;
    }
    String::from_utf8(out).unwrap_or_default()
}

/// An id coming from the frontend that may end up in a path: lowercase, digits, `-` and `_`.
/// Accepts those from `new_id` and the deterministic ones from the migration.
pub fn is_valid_id(id: &str) -> bool {
    (2..=64).contains(&id.len())
        && !id.starts_with('-')
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

pub fn check_id(id: &str, what: &str) -> Result<(), String> {
    if is_valid_id(id) {
        Ok(())
    } else {
        Err(format!("Invalid {what} id: \"{id}\"."))
    }
}

/// Clips to `max` characters (not bytes), appending `…`.
pub fn clip_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests;
