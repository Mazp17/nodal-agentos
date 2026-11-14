#!/usr/bin/env bash
# Runs a worktree's app on its own copy of the dev data and its own port, so its migrations
# never touch the shared dev database. Delete <worktree>/.preview to start from fresh data.
# Usage: scripts/preview.sh [worktree]   (default: the current folder)
set -euo pipefail

wt=$(cd "${1:-.}" && pwd)
# shortcut: macOS path only, add the Linux/Windows ones if a preview is needed there.
src="$HOME/Library/Application Support/io.github.mazp17.nodal.dev"
dir="$wt/.preview"

if [ ! -f "$dir/nodal.db" ]; then
  mkdir -p "$dir"
  # .backup reads a consistent snapshot (WAL included) even while Nodal Dev is open.
  sqlite3 "$src/nodal.db" ".backup '$dir/nodal.db'"
  for sub in tasks runs; do
    [ -d "$src/$sub" ] && cp -R "$src/$sub" "$dir/"
  done
  echo "preview: copied the dev data to $dir"
fi

port=1430
while lsof -iTCP:"$port" -sTCP:LISTEN >/dev/null 2>&1; do port=$((port + 1)); done

cd "$wt"
[ -d node_modules ] || pnpm install --frozen-lockfile
echo "preview: $wt on port $port"
NODAL_DATA_DIR="$dir" NODAL_DEV_PORT="$port" \
  exec pnpm tauri dev --config "{\"build\":{\"devUrl\":\"http://localhost:$port\"}}"
