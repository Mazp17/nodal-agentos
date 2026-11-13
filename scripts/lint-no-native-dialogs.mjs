// Native dialogs do not work under Tauri (plugin-dialog makes them async): fail on any use.
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";

const RE = /(window|globalThis|self)\??\.(confirm|alert|prompt)\s*\(|(^|[^.?\w$])(confirm|alert|prompt)\(/;
let bad = false;
for (const f of readdirSync("src", { recursive: true })) {
  const p = join("src", f);
  if (!/\.[cm]?[jt]sx?$|\.css$|\.html$/.test(p)) continue;
  readFileSync(p, "utf8")
    .split("\n")
    .forEach((line, i) => {
      if (RE.test(line)) {
        console.error(`${p}:${i + 1}:${line}`);
        bad = true;
      }
    });
}
if (bad) {
  console.error("Native dialogs do not work under Tauri (plugin-dialog makes them async). Use useConfirm() from src/ui/ConfirmDialog.");
  process.exit(1);
}
