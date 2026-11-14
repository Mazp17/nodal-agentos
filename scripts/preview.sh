#!/usr/bin/env bash
# Runs a worktree's app on its own copy of the dev data and its own port, so its migrations
# never touch the shared dev database. Delete <worktree>/.preview to start from fresh data.
# Opens on <project>'s board (default: playground).
# Usage: scripts/preview.sh [worktree] [project]   (default: the current folder)
set -euo pipefail

wt=$(cd "${1:-.}" && pwd)
# shortcut: macOS path only, add the Linux/Windows ones if a preview is needed there.
src="$HOME/Library/Application Support/io.github.mazp17.nodal.dev"
dir="$wt/.preview"
project="${2:-playground}"

if [ ! -f "$dir/nodal.db" ]; then
  mkdir -p "$dir"
  # .backup reads a consistent snapshot (WAL included) even while Nodal Dev is open.
  sqlite3 "$src/nodal.db" ".backup '$dir/nodal.db'"
  for sub in tasks runs; do
    [ -d "$src/$sub" ] && cp -R "$src/$sub" "$dir/"
  done
  echo "preview: copied the dev data to $dir"
fi

q="'"
project_id=$(sqlite3 "$dir/nodal.db" \
  "SELECT id FROM projects WHERE name = '${project//$q/$q$q}' AND archived_at IS NULL LIMIT 1")
if [ -z "$project_id" ]; then
  echo "preview: WARNING: no project named '$project' in the dev data, opening on All projects" >&2
fi

port=1430
while lsof -iTCP:"$port" -sTCP:LISTEN >/dev/null 2>&1; do port=$((port + 1)); done

cd "$wt"
[ -d node_modules ] || pnpm install --frozen-lockfile
echo "preview: $wt on port $port"
NODAL_DATA_DIR="$dir" NODAL_DEV_PORT="$port" VITE_PREVIEW_PROJECT_ID="$project_id" \
  exec pnpm tauri dev --config "{\"build\":{\"devUrl\":\"http://localhost:$port\"}}"
