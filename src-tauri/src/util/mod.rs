//! Shared utilities: time, ids, paths, git and blocking I/O.

pub mod git;
pub mod paths;

use std::sync::atomic::{AtomicU32, Ordering};

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Runs `f` on a blocking thread (disk, git) without stalling the runtime.
pub async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(|e| format!("Internal error: {e}"))?
}

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
    format!("{prefix}{}{}{}", base32(now as u64, 10), base32(seq as u64, 3), base32(rnd, 5))
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
        && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

pub fn check_id(id: &str, what: &str) -> Result<(), String> {
    if is_valid_id(id) {
        Ok(())
    } else {
        Err(format!("Invalid {what} id: \"{id}\"."))
    }
}

/// Atomic write: temp file + rename (creates any missing folders).
pub fn write_atomic(path: &std::path::Path, contents: &[u8]) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("Couldn't create {}: {e}", dir.display()))?;
    }
    let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
    std::fs::write(&tmp, contents).map_err(|e| format!("Couldn't write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("Couldn't save {}: {e}", path.display()))
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
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_sortable_and_path_safe() {
        let a = new_id('t', 1_700_000_000_000);
        let b = new_id('t', 1_700_000_000_000);
        let c = new_id('t', 1_700_000_000_001);
        assert_ne!(a, b);
        assert!(a < c && b < c, "{a} {b} {c}");
        assert_eq!(a.len(), 19);
        assert!(is_valid_id(&a), "{a}");
        assert!(new_id('r', 1).starts_with('r'));
        assert!(is_valid_id("legacy-task_1"));
        assert!(!is_valid_id("../etc"));
        assert!(!is_valid_id("t/../x"));
        assert!(!is_valid_id("TABC"));
        assert!(!is_valid_id("-x"));
        assert!(!is_valid_id(""));
    }

    #[test]
    fn clip() {
        assert_eq!(clip_chars("hello", 10), "hello");
        assert_eq!(clip_chars("naïveté", 4), "naï…");
    }
}
