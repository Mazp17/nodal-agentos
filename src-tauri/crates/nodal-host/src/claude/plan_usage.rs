//! % used of the Claude plan's 5-hour and weekly windows, from the `rate_limit_event` lines
//! `claude -p --output-format stream-json` prints (UNDOCUMENTED; observed in Claude Code
//! 2.1.325 on 2026-11-13):
//!
//! ```json
//! {"type":"rate_limit_event","rate_limit_info":{"status":"allowed","resetsAt":1794621600,
//!  "rateLimitType":"five_hour", ...,"unifiedWindows":{
//!   "five_hour":{"utilization":0.12,"resetsAt":1794621600,"status":"allowed"},
//!   "seven_day":{"utilization":0.03,"resetsAt":1794686400,"status":"allowed"}}}}
//! ```
//!
//! `utilization` is 0–1, `resetsAt` epoch seconds. Chats hand every such line to `record`,
//! which keeps the last one in `<nodal home>/plan-usage.json` so it survives restarts.
//!
//! Chosen over the OAuth usage endpoint Claude Code's `/usage` reads: that one needs Claude
//! Code's token and User-Agent from a third-party app and rate-limits hard (a 429 with
//! `Retry-After: 3464` after ~20 calls). The cost: values only refresh when a Nodal agent gets
//! a reply.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageWindow {
    /// % used, 0–100.
    pub pct: f64,
    /// Epoch ms. `None` once it has passed: the window rolled over and the next reset is
    /// unknown until the next event.
    pub resets_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub five_hour: UsageWindow,
    pub weekly: UsageWindow,
    /// Epoch ms of the event.
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum PlanUsage {
    Available(Snapshot),
    Unavailable { reason: String },
}

/// The windows in a `rate_limit_event` line, or `None` if it isn't one / lacks them.
pub fn parse_event(line: &str, now_ms: i64) -> Option<Snapshot> {
    let v: Value = serde_json::from_str(line).ok()?;
    if v.get("type")?.as_str()? != "rate_limit_event" {
        return None;
    }
    let windows = v.get("rate_limit_info")?.get("unifiedWindows")?;
    let window = |key: &str| {
        let w = windows.get(key)?;
        Some(UsageWindow {
            pct: w.get("utilization")?.as_f64()? * 100.0,
            resets_at: w.get("resetsAt").and_then(Value::as_i64).map(|s| s * 1000),
        })
    };
    Some(Snapshot { five_hour: window("five_hour")?, weekly: window("seven_day")?, updated_at: now_ms })
}

/// A window whose reset has passed is back at 0%.
fn roll_over(w: UsageWindow, now_ms: i64) -> UsageWindow {
    match w.resets_at {
        Some(at) if at <= now_ms => UsageWindow { pct: 0.0, resets_at: None },
        _ => w,
    }
}

fn file() -> Result<PathBuf, String> {
    Ok(crate::paths::nodal_home()?.join("plan-usage.json"))
}

fn record_in(path: &Path, line: &str, now_ms: i64) -> Result<(), String> {
    let Some(snap) = parse_event(line, now_ms) else { return Ok(()) };
    let json = serde_json::to_vec(&snap).map_err(|e| e.to_string())?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    // Write + rename: a reader never sees half a file.
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, json).map_err(|e| e.to_string())?;
    fs::rename(&tmp, path).map_err(|e| e.to_string())
}

/// Keeps the windows of a `rate_limit_event` line; anything else is ignored.
pub fn record(line: &str, now_ms: i64) {
    if let Err(e) = file().and_then(|p| record_in(&p, line, now_ms)) {
        tracing::warn!("could not save plan usage: {e}");
    }
}

fn read_in(path: &Path, now_ms: i64) -> PlanUsage {
    let snap = fs::read(path).ok().and_then(|b| serde_json::from_slice::<Snapshot>(&b).ok());
    match snap {
        Some(s) => PlanUsage::Available(Snapshot {
            five_hour: roll_over(s.five_hour, now_ms),
            weekly: roll_over(s.weekly, now_ms),
            updated_at: s.updated_at,
        }),
        None => PlanUsage::Unavailable { reason: "No data yet: it updates when a Nodal chat or task gets a reply".into() },
    }
}

/// Last recorded plan usage. Never fails: no data is an `Unavailable` with a reason.
pub fn read(now_ms: i64) -> PlanUsage {
    match file() {
        Ok(p) => read_in(&p, now_ms),
        Err(reason) => PlanUsage::Unavailable { reason },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Real line (2026-11-13), ids trimmed.
    const REAL: &str = r#"{"type":"rate_limit_event","rate_limit_info":{"status":"allowed","resetsAt":1794621600,"rateLimitType":"five_hour","overageStatus":"rejected","overageDisabledReason":"out_of_credits","isUsingOverage":false,"unifiedWindows":{"five_hour":{"utilization":0.12,"resetsAt":1794621600,"status":"allowed"},"seven_day":{"utilization":0.03,"resetsAt":1794686400,"status":"allowed"}}},"uuid":"u","session_id":"s"}"#;

    #[test]
    fn parses_real_event() {
        let s = parse_event(REAL, 7).unwrap();
        assert!((s.five_hour.pct - 12.0).abs() < 1e-9);
        assert_eq!(s.five_hour.resets_at, Some(1_794_621_600_000));
        assert!((s.weekly.pct - 3.0).abs() < 1e-9);
        assert_eq!(s.weekly.resets_at, Some(1_794_686_400_000));
        assert_eq!(s.updated_at, 7);
    }

    #[test]
    fn missing_fields_or_other_lines_are_ignored() {
        for line in [
            r#"{"type":"rate_limit_event","rate_limit_info":{"status":"allowed"}}"#,
            r#"{"type":"rate_limit_event","rate_limit_info":{"unifiedWindows":{"five_hour":{"utilization":0.1}}}}"#,
            r#"{"type":"result","rate_limit_info":{}}"#,
            "not json",
        ] {
            assert_eq!(parse_event(line, 0), None, "{line}");
        }
        // A missing reset is kept as unknown.
        let s = parse_event(
            r#"{"type":"rate_limit_event","rate_limit_info":{"unifiedWindows":{"five_hour":{"utilization":0.5},"seven_day":{"utilization":1}}}}"#,
            0,
        )
        .unwrap();
        assert_eq!((s.five_hour.resets_at, s.weekly.pct), (None, 100.0));
    }

    fn tmp_file(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("nodal-plan-usage-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir.join("plan-usage.json")
    }

    #[test]
    fn record_then_read_and_roll_over() {
        let path = tmp_file("rw");
        assert!(matches!(read_in(&path, 0), PlanUsage::Unavailable { .. }));
        record_in(&path, r#"{"type":"assistant"}"#, 0).unwrap();
        assert!(!path.exists(), "non-events don't write");

        record_in(&path, REAL, 1).unwrap();
        let PlanUsage::Available(s) = read_in(&path, 1_794_621_599_999) else { panic!() };
        assert!((s.five_hour.pct - 12.0).abs() < 1e-9);
        // Past the 5-hour reset (not the weekly one): 5h back to 0, weekly unchanged.
        let PlanUsage::Available(s) = read_in(&path, 1_794_621_600_000) else { panic!() };
        assert_eq!(s.five_hour, UsageWindow { pct: 0.0, resets_at: None });
        assert_eq!(s.weekly.resets_at, Some(1_794_686_400_000));
        assert_eq!(s.updated_at, 1);
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn corrupt_file_is_unavailable() {
        let path = tmp_file("bad");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "{").unwrap();
        assert!(matches!(read_in(&path, 0), PlanUsage::Unavailable { .. }));
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn serializes_for_the_frontend() {
        let v = serde_json::to_value(PlanUsage::Available(parse_event(REAL, 5).unwrap())).unwrap();
        assert_eq!(v["status"], "available");
        assert_eq!(v["fiveHour"]["resetsAt"], 1_794_621_600_000_i64);
        assert_eq!(v["updatedAt"], 5);
        let v = serde_json::to_value(PlanUsage::Unavailable { reason: "x".into() }).unwrap();
        assert_eq!(v, serde_json::json!({ "status": "unavailable", "reason": "x" }));
    }
}
