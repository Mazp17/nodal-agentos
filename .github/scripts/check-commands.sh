#!/usr/bin/env bash
# Registered Tauri commands must equal the ones the frontend invokes.
set -euo pipefail
root=$(git rev-parse --show-toplevel)
if grep -rnE 'invoke[^(]*\(\s*$' "$root/src"; then echo "invoke(...) name must be on the same line" >&2; exit 1; fi
be=$(awk '/generate_handler!\[/{on=1;next} on&&/\]\)/{exit} on' "$root/src-tauri/src/lib.rs" \
     | sed -E 's://.*$::; s/[[:space:],]//g' | grep -v '^$' | sed -E 's/.*:://' | sort -u)
fe=$(grep -rhoE 'invoke[^(]*\(\s*"[a-z_]+"' "$root/src" | sed -E 's/.*"([a-z_]+)"/\1/' | sort -u)
diff <(echo "$be") <(echo "$fe") && echo "commands OK: $(echo "$be" | wc -l | tr -d ' ')"
