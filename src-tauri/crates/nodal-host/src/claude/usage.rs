//! Token usage of every Claude Code session on this machine, counted the way Claude Code's
//! `/stats` counts it so the two match (READ-ONLY):
//! - All time: `<config>/stats-cache.json` (aggregates through `lastComputedDate`, kept after
//!   old transcripts are deleted) plus the transcripts dated after it.
//! - 7 and 30 days: the transcripts in `<config>/projects`, by UTC date.
//!
//! Like `/stats`, every assistant line with `message.usage` counts, even when one response is
//! split over several lines (one per content block), so totals run ~1.8× the API's.
//!
//! Files are cached by size and mtime: only new or grown transcripts are parsed again.

use std::collections::HashMap;
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
    /// Sessions started in the period (subagents not counted).
    pub sessions: u64,
}

impl UsageTotals {
    fn add(&mut self, e: &Entry) {
        self.input += e.input;
        self.output += e.output;
        self.cache_read += e.cache_read;
        self.cache_write += e.cache_write;
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
    /// Tokens per UTC day, oldest first: `[29]` is today.
    pub daily_30d: Vec<u64>,
    /// Last 30 days by model, most tokens first.
    pub models_30d: Vec<ModelUsage>,
    /// First session (epoch ms).
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

#[derive(Default)]
struct Transcript {
    /// First message of a main-session transcript; `None` for subagents and empty files.
    started: Option<i64>,
    entries: Vec<Entry>,
}

struct Cached {
    len: u64,
    modified: Option<SystemTime>,
    t: Transcript,
}

static CACHE: Mutex<Option<HashMap<PathBuf, Cached>>> = Mutex::new(None);

/// Usage under the Claude config dir as of `now_ms`.
pub fn usage_summary(now_ms: i64) -> Result<UsageSummary, String> {
    let dir = crate::claude::fs::paths::claude_config_dir().ok_or("$HOME is not set.")?;
    Ok(summary_of(&dir, now_ms))
}

fn summary_of(config: &Path, now_ms: i64) -> UsageSummary {
    let files = transcripts(&config.join("projects"));
    let mut guard = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    let cache = guard.get_or_insert_with(HashMap::new);
    cache.retain(|p, _| files.contains(p));
    for path in &files {
        let Ok(meta) = fs::metadata(path) else { continue };
        let (len, modified) = (meta.len(), meta.modified().ok());
        if !cache.get(path).is_some_and(|c| c.len == len && c.modified == modified) {
            let sub = path.components().any(|p| p.as_os_str() == "subagents");
            let t = fs::read_to_string(path).map(|s| parse(&s, sub)).unwrap_or_default();
            cache.insert(path.clone(), Cached { len, modified, t });
        }
    }

    let stats = fs::read_to_string(config.join("stats-cache.json")).ok().and_then(|s| StatsCache::parse(&s));
    let today = now_ms.div_euclid(DAY_MS);
    // `/stats` takes "N days" as from the UTC date of N-1 days ago.
    let from = |n: i64| (now_ms - (n - 1) * DAY_MS).div_euclid(DAY_MS);
    let (d7, d30) = (from(7), from(30));
    let all_from = stats.as_ref().map_or(i64::MIN, |s| s.last_day + 1);

    let mut out = UsageSummary { daily_30d: vec![0; 30], ..Default::default() };
    let mut models: HashMap<&str, UsageTotals> = HashMap::new();
    for c in cache.values() {
        if let Some(at) = c.t.started {
            let day = at.div_euclid(DAY_MS);
            for (t, from) in [(&mut out.last_7d, d7), (&mut out.last_30d, d30), (&mut out.all_time, all_from)] {
                t.sessions += u64::from(day >= from);
            }
            out.last_24h.sessions += u64::from(now_ms - at < DAY_MS);
            out.since = Some(out.since.map_or(at, |s| s.min(at)));
        }
        for e in &c.t.entries {
            let day = e.at.div_euclid(DAY_MS);
            if day >= all_from {
                out.all_time.add(e);
            }
            if day >= d30 {
                out.last_30d.add(e);
                models.entry(&e.model).or_default().add(e);
                if day <= today {
                    out.daily_30d[(29 - (today - day)) as usize] += total(e);
                }
            }
            if day >= d7 {
                out.last_7d.add(e);
            }
            if now_ms - e.at < DAY_MS {
                out.last_24h.add(e);
            }
        }
    }
    if let Some(s) = stats {
        let t = &mut out.all_time;
        t.input += s.totals.input;
        t.output += s.totals.output;
        t.cache_read += s.totals.cache_read;
        t.cache_write += s.totals.cache_write;
        t.sessions += s.totals.sessions;
        out.since = s.since.or(out.since);
    }
    out.models_30d = models.into_iter().map(|(m, totals)| ModelUsage { model: m.to_string(), totals }).collect();
    out.models_30d.sort_by_key(|m| std::cmp::Reverse(sum(&m.totals)));
    out
}

fn total(e: &Entry) -> u64 {
    e.input + e.output + e.cache_read + e.cache_write
}

fn sum(t: &UsageTotals) -> u64 {
    t.input + t.output + t.cache_read + t.cache_write
}

/// The aggregates Claude Code keeps in `stats-cache.json`.
struct StatsCache {
    /// `lastComputedDate` as a UTC day number.
    last_day: i64,
    totals: UsageTotals,
    since: Option<i64>,
}

impl StatsCache {
    fn parse(text: &str) -> Option<Self> {
        let v: Value = serde_json::from_str(text).ok()?;
        let last_day = parse_iso_ms(&format!("{}T00:00:00Z", v["lastComputedDate"].as_str()?))?.div_euclid(DAY_MS);
        let mut totals = UsageTotals { sessions: v["totalSessions"].as_u64().unwrap_or(0), ..Default::default() };
        for m in v["modelUsage"].as_object()?.values() {
            let n = |k: &str| m[k].as_u64().unwrap_or(0);
            totals.input += n("inputTokens");
            totals.output += n("outputTokens");
            totals.cache_read += n("cacheReadInputTokens");
            totals.cache_write += n("cacheCreationInputTokens");
        }
        Some(Self { last_day, totals, since: v["firstSessionDate"].as_str().and_then(parse_iso_ms) })
    }
}

/// `projects/<project>/*.jsonl` and `projects/<project>/<session>/subagents/agent-*.jsonl`,
/// the files `/stats` reads.
fn transcripts(projects: &Path) -> Vec<PathBuf> {
    let jsonl = |dir: &Path, prefix: &str| -> Vec<PathBuf> {
        let Ok(rd) = fs::read_dir(dir) else { return Vec::new() };
        rd.flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
            .map(|e| e.path())
            .filter(|p| {
                let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
                name.ends_with(".jsonl") && name.starts_with(prefix)
            })
            .collect()
    };
    let dirs = |dir: &Path| -> Vec<PathBuf> {
        let Ok(rd) = fs::read_dir(dir) else { return Vec::new() };
        rd.flatten().filter(|e| e.file_type().is_ok_and(|t| t.is_dir())).map(|e| e.path()).collect()
    };
    let mut out = Vec::new();
    for project in dirs(projects) {
        out.extend(jsonl(&project, ""));
        for session in dirs(&project) {
            out.extend(jsonl(&session.join("subagents"), "agent-"));
        }
    }
    out
}

/// One transcript: its start and every usage line. Main sessions skip sidechain lines.
fn parse(text: &str, subagent: bool) -> Transcript {
    let mut t = Transcript::default();
    let mut first: Option<i64> = None;
    for line in text.lines() {
        // Cheap filter before parsing: message lines all carry a timestamp.
        if !line.contains("\"timestamp\"") {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        if !matches!(v["type"].as_str(), Some("user" | "assistant" | "attachment" | "system")) {
            continue;
        }
        if !subagent && v["isSidechain"] == true {
            continue;
        }
        let Some(at) = v["timestamp"].as_str().and_then(parse_iso_ms) else { continue };
        first.get_or_insert(at);
        let msg = &v["message"];
        let u = &msg["usage"];
        if v["type"] != "assistant" || !u.is_object() {
            continue;
        }
        let model = msg["model"].as_str().unwrap_or("unknown");
        if model == "<synthetic>" {
            continue;
        }
        let n = |k: &str| u[k].as_u64().unwrap_or(0);
        t.entries.push(Entry {
            at,
            model: model.to_string(),
            input: n("input_tokens"),
            output: n("output_tokens"),
            cache_read: n("cache_read_input_tokens"),
            cache_write: n("cache_creation_input_tokens"),
        });
    }
    if !subagent {
        t.started = first;
    }
    t
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
    fn counts_like_stats() {
        let line = |id: &str, ts: &str, out: u64, extra: &str| {
            format!(
                r#"{{"type":"assistant","requestId":"r-{id}","timestamp":"{ts}"{extra},"message":{{"id":"{id}","model":"m","usage":{{"input_tokens":2,"output_tokens":{out},"cache_read_input_tokens":10,"cache_creation_input_tokens":5}}}}}}"#
            )
        };
        let main = [
            r#"{"type":"user","timestamp":"2026-11-01T00:00:00Z","message":{"role":"user","content":"hi"}}"#.to_string(),
            line("b", "2026-11-01T00:00:01Z", 3, ""),
            // One response over two lines: `/stats` counts both.
            line("a", "2026-11-12T00:00:00Z", 7, ""),
            line("a", "2026-11-12T00:00:01Z", 7, ""),
            line("s", "2026-11-12T00:00:02Z", 100, r#","isSidechain":true"#),
        ]
        .join("\n");
        let dir = std::env::temp_dir().join(format!("nodal-usage-{}", std::process::id()));
        fs::create_dir_all(dir.join("projects/p/s1/subagents")).unwrap();
        fs::write(dir.join("projects/p/s1.jsonl"), main).unwrap();
        fs::write(dir.join("projects/p/s1/subagents/agent-x.jsonl"), line("c", "2026-11-12T01:00:00Z", 1, "")).unwrap();
        // Through Nov 5: 1000 tokens of each kind over 40 sessions.
        fs::write(
            dir.join("stats-cache.json"),
            r#"{"lastComputedDate":"2026-11-05","totalSessions":40,"firstSessionDate":"2026-02-09T17:41:08.949Z",
               "modelUsage":{"m":{"inputTokens":1000,"outputTokens":1000,"cacheReadInputTokens":1000,"cacheCreationInputTokens":1000}}}"#,
        )
        .unwrap();
        let now = parse_iso_ms("2026-11-12T12:00:00Z").unwrap();
        let s = summary_of(&dir, now);
        fs::remove_dir_all(&dir).ok();

        // Nov 12: two lines of "a" plus the subagent; the sidechain line is skipped.
        assert_eq!(s.last_7d, UsageTotals { input: 6, output: 15, cache_read: 30, cache_write: 15, sessions: 0 });
        assert_eq!(s.last_30d, UsageTotals { input: 8, output: 18, cache_read: 40, cache_write: 20, sessions: 1 });
        // The Nov 1 session is already in the stats cache.
        assert_eq!(s.all_time, UsageTotals { input: 1006, output: 1015, cache_read: 1030, cache_write: 1015, sessions: 40 });
        assert_eq!((s.daily_30d[29], s.daily_30d[18]), (66, 20));
        assert_eq!(s.since, parse_iso_ms("2026-02-09T17:41:08.949Z"));
    }
}
