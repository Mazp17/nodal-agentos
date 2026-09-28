//! Task provider integration: state mapping, plan rendering, sync decisions and import
//! routing (all pure).

pub mod decisions;
pub mod plan_render;
pub mod routing;
pub mod state_map;

/// Epoch ms → `YYYY-MM-DDTHH:MM:SS.mmmZ` (UTC), without depending on chrono.
pub fn iso_from_ms(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let millis = ms.rem_euclid(1000);
    let days = secs.div_euclid(86_400);
    let tod = secs.rem_euclid(86_400);
    // Howard Hinnant's "civil from days" algorithm.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{millis:03}Z", tod / 3600, tod % 3600 / 60, tod % 60)
}

/// Shorter keys show no hint: the last 4 would be too much of the key.
const KEY_HINT_MIN_CHARS: usize = 12;

/// Last 4 characters of the key to recognize it in the UI; never the whole key.
pub fn key_hint(key: &str) -> Option<String> {
    let chars: Vec<char> = key.trim().chars().collect();
    (chars.len() >= KEY_HINT_MIN_CHARS).then(|| chars[chars.len() - 4..].iter().collect())
}

#[cfg(test)]
mod tests;
