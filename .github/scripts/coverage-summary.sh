#!/usr/bin/env bash
# Turns the JSON from `cargo llvm-cov --json --summary-only` into the Markdown posted on PRs.
# Usage: coverage-summary.sh <coverage.json> <min line %>
set -euo pipefail
json=$1
min=$2

jq -r --arg min "$min" '
  def pct: . * 10 | round / 10 | tostring + "%";
  .data[0] as $d
  | ($d.totals.lines.percent >= ($min | tonumber)) as $ok
  | "<!-- nodal-coverage -->",
    "## Coverage (Rust backend)",
    "",
    "**Lines: \($d.totals.lines.percent | pct)** \(if $ok then "✅" else "❌" end) (minimum \($min)%) · Regions: \($d.totals.regions.percent | pct) · Functions: \($d.totals.functions.percent | pct)",
    "",
    "<details><summary>Per file (lowest first)</summary>",
    "",
    "| File | Lines | Covered |",
    "|---|---:|---:|",
    ($d.files
      | sort_by(.summary.lines.percent)[]
      | "| `\(.filename | sub("^.*/src-tauri/"; ""))` | \(.summary.lines.percent | pct) | \(.summary.lines.covered)/\(.summary.lines.count) |"),
    "",
    "</details>",
    "",
    "_The frontend has no tests yet and is not measured._"
' "$json"
