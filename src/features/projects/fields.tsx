// Controles compartidos por el onboarding, New project y Project settings.

import { useEffect, useId, useState, type KeyboardEvent } from "react";
import { providerScopes, providerSetKey } from "../../domain/api";
import type { ScopeRef } from "../../domain/types";
import { isConnected, LINEAR, useProviderStatus } from "../../shell/providerStatus";
import { PROJECT_COLORS } from "./create";
import "./projects.css";

/** Nombres accesibles de `PROJECT_COLORS`, en el mismo orden. */
const COLOR_NAMES = ["Amber", "Blue", "Green", "Magenta", "Yellow", "Red", "Teal", "Violet"];

export function ColorSwatches({
  value,
  onChange,
  label = "Color",
}: {
  value: string;
  onChange: (c: string) => void;
  label?: string;
}) {
  const colors: readonly string[] = PROJECT_COLORS.includes(value as (typeof PROJECT_COLORS)[number])
    ? PROJECT_COLORS
    : [...PROJECT_COLORS, value];
  // Radio group con flechas (WAI-ARIA): un solo tab stop.
  const onKey = (e: KeyboardEvent<HTMLDivElement>) => {
    const dir = e.key === "ArrowRight" || e.key === "ArrowDown" ? 1 : e.key === "ArrowLeft" || e.key === "ArrowUp" ? -1 : 0;
    if (!dir) return;
    e.preventDefault();
    const i = Math.max(0, colors.indexOf(value));
    const next = colors[(i + dir + colors.length) % colors.length]!;
    onChange(next);
    const el = e.currentTarget.querySelector<HTMLElement>(`[data-color="${CSS.escape(next)}"]`);
    el?.focus();
  };
  return (
    <div className="swatches" role="radiogroup" aria-label={label} onKeyDown={onKey}>
      {colors.map((c, k) => (
        <button
          key={c}
          type="button"
          role="radio"
          data-color={c}
          aria-checked={c === value}
          aria-label={COLOR_NAMES[k] ?? `Custom color`}
          tabIndex={c === value || (!colors.includes(value) && k === 0) ? 0 : -1}
          className="swatch"
          style={{ background: c }}
          onClick={() => onChange(c)}
        />
      ))}
    </div>
  );
}

export interface SegOption<T> {
  value: T;
  label: string;
}

/** Segmentado de una sola elección (radio group). */
export function Segmented<T>({
  value,
  options,
  onChange,
  label,
  disabled,
}: {
  value: T;
  options: SegOption<T>[];
  onChange: (v: T) => void;
  label: string;
  disabled?: boolean;
}) {
  const current = Math.max(
    0,
    options.findIndex((o) => o.value === value),
  );
  const onKey = (e: KeyboardEvent<HTMLDivElement>) => {
    const dir = e.key === "ArrowRight" ? 1 : e.key === "ArrowLeft" ? -1 : 0;
    if (!dir || disabled) return;
    e.preventDefault();
    const n = (current + dir + options.length) % options.length;
    onChange(options[n]!.value);
    (e.currentTarget.children[n] as HTMLElement | undefined)?.focus();
  };
  return (
    <div className="seg" role="radiogroup" aria-label={label} onKeyDown={onKey}>
      {options.map((o, k) => (
        <button
          key={`${String(o.value)}-${k}`}
          type="button"
          role="radio"
          aria-checked={o.value === value}
          tabIndex={k === current ? 0 : -1}
          className="seg-opt"
          disabled={disabled}
          onClick={() => onChange(o.value)}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}

/** Teams de Linear (si la key está conectada) para elegir el scope de una fuente. */
export function useLinearTeams(enabled: boolean) {
  const [teams, setTeams] = useState<ScopeRef[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    if (!enabled) return;
    let alive = true;
    setError(null);
    providerScopes(LINEAR)
      .then((s) => alive && setTeams(s.filter((x) => x.kind === "team")))
      .catch((e) => alive && setError(String(e)));
    return () => {
      alive = false;
    };
  }, [enabled]);
  return { teams, error };
}

/**
 * Bloque "Linear" del onboarding y de New project: si no hay key, la pide (o remite a
 * Settings con `allowKey=false`); si hay, deja elegir el team.
 */
export function LinearTeamPicker({
  value,
  onChange,
  allowKey,
}: {
  value: ScopeRef | null;
  onChange: (s: ScopeRef | null) => void;
  allowKey: boolean;
}) {
  const status = useProviderStatus();
  const connected = isConnected(status.linear);
  const { teams, error } = useLinearTeams(connected);
  const [key, setKey] = useState("");
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState<{ text: string; error: boolean } | null>(null);
  const keyId = useId();

  // Primer team por defecto al conectar.
  useEffect(() => {
    if (teams && teams.length && !value) onChange(teams[0]!);
  }, [teams, value, onChange]);

  const saveKey = async () => {
    const k = key.trim();
    if (!k) return;
    setBusy(true);
    setMsg({ text: "Testing against api.linear.app…", error: false });
    try {
      const s = await providerSetKey(LINEAR, k);
      status.set(s);
      if (s.error) {
        setMsg({ text: s.error, error: true });
      } else {
        setKey("");
        setMsg(null);
      }
    } catch (e) {
      setMsg({ text: String(e), error: true });
    } finally {
      setBusy(false);
    }
  };

  if (!connected) {
    if (!allowKey) {
      return (
        <span className="field-hint">
          {status.linear?.hasKey && status.linear.error
            ? `Linear can't be reached: ${status.linear.error}. You can add a source later.`
            : "Connect Linear in Settings → Integrations first. You can add a source later."}
        </span>
      );
    }
    return (
      <div className="field">
        <label className="sr-only" htmlFor={keyId}>
          Linear API key
        </label>
        <div className="linear-key-row">
          <input
            id={keyId}
            className="input input-mono"
            type="password"
            autoComplete="off"
            spellCheck={false}
            placeholder="lin_api_…"
            value={key}
            onChange={(e) => {
              setKey(e.target.value);
              setMsg(null);
            }}
            onKeyDown={(e) => e.key === "Enter" && void saveKey()}
          />
          <button type="button" className="btn btn-lg" disabled={busy || !key.trim()} onClick={() => void saveKey()}>
            {busy ? "Testing…" : "Connect"}
          </button>
        </div>
        <span className={msg?.error ? "field-error" : "field-hint"} role={msg?.error ? "alert" : undefined}>
          {msg?.text ?? "Personal API key from Linear → Settings → API. Stored in the macOS Keychain."}
        </span>
      </div>
    );
  }

  return (
    <div className="field">
      <span className="linear-connected">
        <span className="dot dot-sm tone-ok" aria-hidden />
        Connected{status.linear?.viewer ? ` as ${status.linear.viewer}` : ""}
      </span>
      {error && (
        <span className="field-error" role="alert">
          {error}
        </span>
      )}
      {!error && teams === null && <span className="field-hint">Loading teams…</span>}
      {teams && teams.length === 0 && <span className="field-hint">No teams in this workspace.</span>}
      {teams && teams.length > 0 && (
        <div className="inline-field">
          <span className="inline-field-label">Team</span>
          <Segmented
            label="Linear team"
            value={value?.id ?? null}
            options={teams.map((t) => ({ value: t.id as string | null, label: t.name }))}
            onChange={(id) => onChange(teams.find((t) => t.id === id) ?? null)}
          />
        </div>
      )}
    </div>
  );
}
