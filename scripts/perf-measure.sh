#!/usr/bin/env bash
# Samples CPU%, RSS and child-process count of a running Nodal process (and its whole
# process tree: `claude`, `git`, `osascript`, the pty helper, etc.) at a fixed interval,
# writing one CSV row per sample. If given the app's log file (started with `RUST_LOG=info`
# or higher), also reads the exact `claude`/`git` spawn and IPC-event counters this branch's
# `tracing` instrumentation logs once a minute and once at exit
# (`target=nodal::spawn`/`nodal::ipc` in src-tauri/src/telemetry.rs) — so "spawns" isn't
# just a `ps`-diff approximation, which misses children that start and exit between samples.
#
# CAN'T be run by an agent: it needs the packaged (or `tauri dev`) app open and driven
# through the UI (Board idle, a run in Runs, the diff drawer, a streaming chat — the S0-S3
# scenarios), and CPU/RSS this way is `ps`-based, not `dtrace`/`powermetrics` (those need
# sudo). Meant for a human to run alongside a manual smoke test.
#
# Usage:
#   scripts/perf-measure.sh -p <pid>  [-d seconds] [-i seconds] [-l app.log] [-o out.csv]
#   scripts/perf-measure.sh -n <name> [-d seconds] [-i seconds] [-l app.log] [-o out.csv]
#
#   -p PID       Measure this pid (and its descendants).
#   -n NAME      Resolve the pid with `pgrep -x NAME` instead (e.g. `nodal`).
#   -d SECONDS   Total duration (default 60).
#   -i SECONDS   Sampling interval (default 1).
#   -l FILE      The app's stderr log (only useful if it was started with `RUST_LOG=info` or
#                higher); appends exact spawn/IPC counters parsed from it to the summary.
#   -o FILE      CSV output path (default: perf-measure-<pid>-<epoch>.csv in the cwd).
#
# Step by step (example: S0 idle baseline):
#   1. RUST_LOG=info pnpm tauri dev 2> /tmp/nodal-s0.log &
#   2. Wait for the window, sit idle on Board (no run active).
#   3. scripts/perf-measure.sh -n nodal -d 60 -l /tmp/nodal-s0.log -o /tmp/s0-idle.csv
#   4. Ctrl-C the app, then read the CSV (avg/max %CPU, RSS) and the counters summary
#      printed at the end.
# Repeat for S1 (a run going), S2 (open the diff drawer), S3 (a chat streaming).

set -euo pipefail

PID=""
NAME=""
DURATION=60
INTERVAL=1
LOGFILE=""
OUTFILE=""

usage() {
    sed -n '2,29p' "$0" | sed 's/^# \{0,1\}//'
    exit "${1:-0}"
}

while getopts "p:n:d:i:l:o:h" opt; do
    case "$opt" in
        p) PID="$OPTARG" ;;
        n) NAME="$OPTARG" ;;
        d) DURATION="$OPTARG" ;;
        i) INTERVAL="$OPTARG" ;;
        l) LOGFILE="$OPTARG" ;;
        o) OUTFILE="$OPTARG" ;;
        h) usage 0 ;;
        *) usage 1 ;;
    esac
done

if [[ -z "$PID" && -z "$NAME" ]]; then
    echo "error: pass -p <pid> or -n <process name>" >&2
    usage 1
fi

if [[ -z "$PID" ]]; then
    PID="$(pgrep -x "$NAME" | head -n1 || true)"
    if [[ -z "$PID" ]]; then
        echo "error: no running process named '$NAME' (pgrep -x)" >&2
        exit 1
    fi
fi

if ! kill -0 "$PID" 2>/dev/null; then
    echo "error: pid $PID is not running" >&2
    exit 1
fi

OUTFILE="${OUTFILE:-perf-measure-${PID}-$(date +%s).csv}"

echo "Measuring pid $PID (and descendants) for ${DURATION}s every ${INTERVAL}s -> $OUTFILE" >&2

# All descendants of $1, recursively (BFS over `pgrep -P`), one pid per line. macOS's `ps`
# has no portable "process tree" flag we can rely on in a script, so this is done by hand.
# Plain functions/files only (no associative arrays, no `mapfile`): the system `/bin/bash` on
# macOS is 3.2 (GPLv2-last), and this script must run without a Homebrew bash installed.
descendants_of() {
    local root="$1"
    local frontier="$root"
    local next child
    printf '%s\n' "$root"
    while [[ -n "$frontier" ]]; do
        next=""
        for p in $frontier; do
            while IFS= read -r child; do
                [[ -n "$child" ]] || continue
                printf '%s\n' "$child"
                next="$next $child"
            done < <(pgrep -P "$p" 2>/dev/null || true)
        done
        frontier="$next"
    done
}

echo "timestamp,elapsed_s,pid_count,tree_cpu_pct,tree_rss_kb,new_pids_this_tick,cumulative_new_pids" > "$OUTFILE"

seen_file="$(mktemp)"
trap 'rm -f "$seen_file"' EXIT
: > "$seen_file"

cumulative_new=0
start_ts=$(date +%s)
end_ts=$((start_ts + DURATION))

while (( $(date +%s) < end_ts )); do
    now=$(date +%s)
    elapsed=$((now - start_ts))

    tree="$(descendants_of "$PID")"
    tree_count=$(printf '%s\n' "$tree" | grep -c '^[0-9]')
    if (( tree_count == 0 )); then
        echo "$(date -u +%Y-%m-%dT%H:%M:%SZ),$elapsed,0,0,0,0,$cumulative_new" >> "$OUTFILE"
        sleep "$INTERVAL"
        continue
    fi

    new_this_tick=0
    while IFS= read -r p; do
        [[ -n "$p" ]] || continue
        if ! grep -qx "$p" "$seen_file"; then
            echo "$p" >> "$seen_file"
            new_this_tick=$((new_this_tick + 1))
        fi
    done <<< "$tree"
    cumulative_new=$((cumulative_new + new_this_tick))

    # Sum %cpu and rss (KB) across the whole tree in one `ps` call.
    tree_csv="$(printf '%s' "$tree" | paste -sd, -)"
    read -r cpu_sum rss_sum < <(
        ps -o pcpu=,rss= -p "$tree_csv" 2>/dev/null \
            | awk '{cpu+=$1; rss+=$2} END {printf "%.1f %d\n", cpu, rss}'
    )

    echo "$(date -u +%Y-%m-%dT%H:%M:%SZ),$elapsed,$tree_count,$cpu_sum,$rss_sum,$new_this_tick,$cumulative_new" >> "$OUTFILE"
    sleep "$INTERVAL"
done

echo "" >&2
echo "Done. CSV: $OUTFILE" >&2
echo "ps-based spawn estimate (new pids seen across samples; misses children that start" >&2
echo "and exit between two ticks — use -l for the exact counters instead): $cumulative_new" >&2

if [[ -n "$LOGFILE" ]]; then
    if [[ ! -r "$LOGFILE" ]]; then
        echo "warning: log file '$LOGFILE' isn't readable, skipping counters summary" >&2
    else
        echo "" >&2
        echo "Exact counters from '$LOGFILE' (target=nodal::spawn / target=nodal::ipc log lines;" >&2
        echo "cumulative since the app started, not windowed to this run — start the app right" >&2
        echo "before measuring for the numbers to line up):" >&2
        for field in claude_spawns git_spawns notify_calls chat_emit_calls; do
            last="$(grep -oE "${field}=[0-9]+" "$LOGFILE" | tail -n1 || true)"
            echo "  ${field}: ${last#*=}" >&2
        done
    fi
fi
