// Preferencias de UI en localStorage, todas con prefijo `nodal.`.

const PREFIX = "nodal.";
const LEGACY_PREFIX = "agent-desk.";
const MIGRATED_FLAG = `${PREFIX}storageMigrated`;

/**
 * Una sola vez: copia las claves `agent-desk.*` a `nodal.*` (sin pisar las nuevas) y
 * borra las viejas. Idempotente; si localStorage no está disponible, no hace nada.
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
    /* localStorage no disponible */
  }
}

/** `key` sin prefijo: `readPref("route")` lee `nodal.route`. */
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
    /* la preferencia no se recuerda */
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
