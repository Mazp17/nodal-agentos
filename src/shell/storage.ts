// UI preferences in localStorage, all prefixed with `nodal.`.

const PREFIX = "nodal.";
const LEGACY_PREFIX = "agent-desk.";
const MIGRATED_FLAG = `${PREFIX}storageMigrated`;

/**
 * One-off: copies the `agent-desk.*` keys to `nodal.*` (without overwriting new ones) and
 * deletes the old ones. Idempotent; if localStorage is unavailable, does nothing.
 */
export function migrateLegacyStorage() {
  try {
    if (localStorage.getItem(MIGRATED_FLAG)) return;
    const legacy: string[] = [];
    for (let i = 0; i < localStorage.length; i++) {
      const k = localStorage.key(i);
      if (k?.startsWith(LEGACY_PREFIX)) legacy.push(k);
    }
    for (const k of legacy) {
      const next = PREFIX + k.slice(LEGACY_PREFIX.length);
      const v = localStorage.getItem(k);
      if (v !== null && localStorage.getItem(next) === null) localStorage.setItem(next, v);
      localStorage.removeItem(k);
    }
    localStorage.setItem(MIGRATED_FLAG, "1");
  } catch {
    /* localStorage unavailable */
  }
}

/** `key` without prefix: `readPref("route")` reads `nodal.route`. */
export function readPref(key: string): string | null {
  try {
    return localStorage.getItem(PREFIX + key);
  } catch {
    return null;
  }
}

export function writePref(key: string, value: string | null) {
  try {
    if (value === null) localStorage.removeItem(PREFIX + key);
    else localStorage.setItem(PREFIX + key, value);
  } catch {
    /* the preference is not remembered */
  }
}

export function readJsonPref<T>(key: string, fallback: T, valid: (v: unknown) => v is T): T {
  const raw = readPref(key);
  if (!raw) return fallback;
  try {
    const v: unknown = JSON.parse(raw);
    return valid(v) ? v : fallback;
  } catch {
    return fallback;
  }
}
