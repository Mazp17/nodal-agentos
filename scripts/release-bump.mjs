#!/usr/bin/env node
// Sets the app version everywhere it lives: package.json (which tauri.conf.json reads),
// src-tauri/Cargo.toml and the crate's entry in src-tauri/Cargo.lock.
// Usage: pnpm release:bump 0.2.0
import { readFileSync, writeFileSync } from "node:fs";

const version = process.argv[2];
if (!/^\d+\.\d+\.\d+$/.test(version ?? "")) {
  console.error("Usage: pnpm release:bump <x.y.z>");
  process.exit(1);
}

function update(path, pattern, replacement) {
  const text = readFileSync(path, "utf8");
  if (!pattern.test(text)) {
    console.error(`Could not find the version in ${path}`);
    process.exit(1);
  }
  writeFileSync(path, text.replace(pattern, replacement));
}

update("package.json", /^(\s*"version":\s*")[^"]+(")/m, `$1${version}$2`);
update("src-tauri/Cargo.toml", /^(\[package\][\s\S]*?^version\s*=\s*")[^"]+(")/m, `$1${version}$2`);
update("src-tauri/Cargo.lock", /^(name = "nodal"\nversion = ")[^"]+(")/m, `$1${version}$2`);

console.log(`Version set to ${version}. Next: move [Unreleased] in CHANGELOG.md to [${version}].`);
