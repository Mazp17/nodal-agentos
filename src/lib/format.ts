/** 1.2k · 108k · 1.24M (mismo criterio que el diseño). */
export function formatTokens(n: number | null | undefined): string {
  if (n == null) return "—";
  if (n >= 1e6) return `${(n / 1e6).toFixed(2)}M`;
  if (n >= 1e3) return `${(n / 1e3).toFixed(n < 1e4 ? 1 : 0)}k`;
  return String(n);
}

/** m:ss por debajo de una hora; si no, `1h 05m`. */
export function formatDuration(ms: number | null | undefined): string {
  if (ms == null) return "—";
  const s = Math.max(0, Math.floor(ms / 1000));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const x = s % 60;
  return h ? `${h}h ${String(m).padStart(2, "0")}m` : `${m}:${String(x).padStart(2, "0")}`;
}

export function formatClock(ms: number): string {
  return new Date(ms).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

export function formatDateTime(ms: number): string {
  return new Date(ms).toLocaleString([], { dateStyle: "medium", timeStyle: "short" });
}

/** Iniciales para el avatar: "Marta Ríos" → "MR". */
export function initials(name: string | null | undefined): string {
  if (!name) return "";
  const parts = name.trim().split(/[\s._-]+/).filter(Boolean);
  const letters = parts.length > 1 ? parts[0][0] + parts[parts.length - 1][0] : name.slice(0, 2);
  return letters.toUpperCase();
}

/** Tono estable por nombre para avatares (misma luminancia, distinto matiz). */
export function avatarHue(name: string): number {
  let h = 0;
  for (const c of name) h = (h * 31 + c.charCodeAt(0)) % 360;
  return h;
}

export function localGet(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

export function localSet(key: string, value: string) {
  try {
    localStorage.setItem(key, value);
  } catch {
    /* localStorage no disponible: la preferencia no se recuerda */
  }
}
