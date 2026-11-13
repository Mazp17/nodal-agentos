//! Token usage of every Claude Code session on this machine, read from the transcripts in
//! `<config>/projects/**/*.jsonl` (READ-ONLY). Each assistant line carries `message.usage`;
//! one message spans several lines (one per content block), so it's counted once per
//! `message.id` + `requestId`.
//!
//! Files are cached by size and mtime: only new or grown transcripts are parsed again.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

use serde::Serialize;
use serde_json::Value;

const DAY_MS: i64 = 24 * 60 * 60 * 1000;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageTotals {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    /// Assistant messages (API responses).
    pub messages: u64,
}

impl UsageTotals {
    fn add(&mut self, e: &Entry) {
        self.input += e.input;
        self.output += e.output;
        self.cache_read += e.cache_read;
        self.cache_write += e.cache_write;
        self.messages += 1;
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelUsage {
    pub model: String,
    pub totals: UsageTotals,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageSummary {
    pub last_24h: UsageTotals,
    pub last_7d: UsageTotals,
    pub last_30d: UsageTotals,
    pub all_time: UsageTotals,
    /// Tokens per rolling day, oldest first: `[29]` is the last 24 hours.
    pub daily_30d: Vec<u64>,
    /// Last 30 days by model, most tokens first.
    pub models_30d: Vec<ModelUsage>,
    pub sessions: u64,
    /// Oldest message seen (epoch ms).
    pub since: Option<i64>,
}

#[derive(Debug, Clone)]
struct Entry {
    at: i64,
    model: String,
    input: u64,
    output: u64,
    cache_read: u64,
    cache_write: u64,
}

struct Cached {
    len: u64,
    modified: Option<SystemTime>,
    entries: Vec<Entry>,
}

static CACHE: Mutex<Option<HashMap<PathBuf, Cached>>> = Mutex::new(None);

/// Usage under `<claude config>/projects` as of `now_ms`.
pub fn usage_summary(now_ms: i64) -> Result<UsageSummary, String> {
    let dir = crate::claude::fs::paths::claude_config_dir().ok_or("$HOME is not set.")?.join("projects");
    Ok(summary_of(&dir, now_ms))
}

fn summary_of(projects: &Path, now_ms: i64) -> UsageSummary {
    let mut files = Vec::new();
    collect_jsonl(projects, &mut files);
    let mut guard = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    let cache = guard.get_or_insert_with(HashMap::new);
    cache.retain(|p, _| files.contains(p));

    let mut out = UsageSummary { daily_30d: vec![0; 30], ..Default::default() };
    let mut models: HashMap<&str, UsageTotals> = HashMap::new();
    for path in &files {
        let Ok(meta) = fs::metadata(path) else { continue };
        let (len, modified) = (meta.len(), meta.modified().ok());
        let fresh = cache.get(path).is_some_and(|c| c.len == len && c.modified == modified);
        if !fresh {
            let entries = fs::read_to_string(path).map(|t| parse(&t)).unwrap_or_default();
            cache.insert(path.clone(), Cached { len, modified, entries });
        }
    }
    for path in &files {
        let Some(c) = cache.get(path) else { continue };
        // Subagent transcripts live under their parent session's folder.
        if !c.entries.is_empty() && !path.components().any(|p| p.as_os_str() == "subagents") {
            out.sessions += 1;
        }
        for e in &c.entries {
            let age = now_ms - e.at;
            out.all_time.add(e);
            if age < 30 * DAY_MS {
                out.last_30d.add(e);
                models.entry(&e.model).or_default().add(e);
                if age >= 0 {
                    out.daily_30d[29 - (age / DAY_MS) as usize] += e.input + e.output + e.cache_read + e.cache_write;
                }
            }
            if age < 7 * DAY_MS {
                out.last_7d.add(e);
            }
            if age < DAY_MS {
                out.last_24h.add(e);
            }
            out.since = Some(out.since.map_or(e.at, |s| s.min(e.at)));
        }
    }
    out.models_30d = models.into_iter().map(|(m, totals)| ModelUsage { model: m.to_string(), totals }).collect();
    out.models_30d.sort_by_key(|m| std::cmp::Reverse(total(&m.totals)));
    out
}

fn total(t: &UsageTotals) -> u64 {
    t.input + t.output + t.cache_read + t.cache_write
}

fn collect_jsonl(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let Ok(ft) = e.file_type() else { continue };
        let p = e.path();
        if ft.is_dir() {
            collect_jsonl(&p, out);
        } else if ft.is_file() && p.extension().is_some_and(|x| x == "jsonl") {
            out.push(p);
        }
    }
}

/// Usage entries of one transcript, one per API response.
fn parse(text: &str) -> Vec<Entry> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for line in text.lines() {
        // Cheap filter before parsing: most lines are user turns and tool results.
        if !line.contains("\"usage\"") || !line.contains("\"assistant\"") {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        if v["type"] != "assistant" {
            continue;
        }
        let msg = &v["message"];
        let u = &msg["usage"];
        if !u.is_object() {
            continue;
        }
        let key = format!("{}:{}", msg["id"].as_str().unwrap_or(""), v["requestId"].as_str().unwrap_or(""));
        if key != ":" && !seen.insert(key) {
            continue;
        }
        let Some(at) = v["timestamp"].as_str().and_then(parse_iso_ms) else { continue };
        let model = msg["model"].as_str().unwrap_or("unknown");
        // Synthetic messages (errors, interruptions) carry zeroed usage.
        if model == "<synthetic>" {
            continue;
        }
        let n = |k: &str| u[k].as_u64().unwrap_or(0);
        out.push(Entry {
            at,
            model: model.to_string(),
            input: n("input_tokens"),
            output: n("output_tokens"),
            cache_read: n("cache_read_input_tokens"),
            cache_write: n("cache_creation_input_tokens"),
        });
    }
    out
}

/// `YYYY-MM-DDTHH:MM:SS[.fff]Z` (UTC, as Claude Code writes it) → epoch ms.
fn parse_iso_ms(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() < 19 || b[4] != b'-' || b[7] != b'-' || b[10] != b'T' {
        return None;
    }
    let num = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    let (y, mo, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (h, mi, sec) = (num(11..13)?, num(14..16)?, num(17..19)?);
    let ms = if b.get(19) == Some(&b'.') { s.get(20..23).and_then(|f| f.parse::<i64>().ok()).unwrap_or(0) } else { 0 };
    // Days from civil (Howard Hinnant).
    let y = if mo <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (mo + if mo > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(((days * 24 + h) * 60 + mi) * 60_000 + sec * 1000 + ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_to_epoch() {
        assert_eq!(parse_iso_ms("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_iso_ms("2026-11-12T23:30:00.250Z"), Some(1_794_526_200_250));
        assert_eq!(parse_iso_ms("nope"), None);
    }

    #[test]
    fn counts_each_message_once() {
        let line = |id: &str, ts: &str, out: u64| {
            format!(
                r#"{{"type":"assistant","requestId":"r-{id}","timestamp":"{ts}","message":{{"id":"{id}","model":"m","usage":{{"input_tokens":2,"output_tokens":{out},"cache_read_input_tokens":10,"cache_creation_input_tokens":5}}}}}}"#
            )
        };
        let text = [
            line("a", "2026-11-12T00:00:00Z", 7),
            line("a", "2026-11-12T00:00:01Z", 7),
            r#"{"type":"user","message":{"role":"user","content":"hi"}}"#.to_string(),
            line("b", "2026-11-01T00:00:00Z", 3),
        ]
        .join("\n");
        let dir = std::env::temp_dir().join(format!("nodal-usage-{}", std::process::id()));
        fs::create_dir_all(dir.join("p")).unwrap();
        fs::write(dir.join("p/s.jsonl"), text).unwrap();
        let now = parse_iso_ms("2026-11-12T12:00:00Z").unwrap();
        let s = summary_of(&dir, now);
        fs::remove_dir_all(&dir).ok();
        assert_eq!(s.all_time, UsageTotals { input: 4, output: 10, cache_read: 20, cache_write: 10, messages: 2 });
        assert_eq!(s.last_24h.messages, 1);
        assert_eq!(s.last_7d.output, 7);
        assert_eq!(s.sessions, 1);
        assert_eq!(s.models_30d.len(), 1);
        assert_eq!((s.daily_30d[29], s.daily_30d[18]), (24, 20));
    }
}
