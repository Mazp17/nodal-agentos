// Controls shared by onboarding, New project and Project settings.

import { useId, type KeyboardEvent } from "react";
import { useProviderScopes, useProviderStatus } from "../../domain/hooks/providers";
import type { ScopeRef } from "../../domain/types";
import { ScopePicker } from "../providers/ScopePicker";
import "../providers/providers.css";
import { PROJECT_COLORS } from "./create";
import "./projects.css";

/** Accessible names for `PROJECT_COLORS`, in the same order. */
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
  // Radio group with arrow keys (WAI-ARIA): a single tab stop.
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

/** Single-choice segmented control (radio group). */
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

/**
 * Linear team or project for a new project's source ("New project" dialog).
 * Without a connected key it points to Settings → Integrations; the key is entered in onboarding
 * (ConnectProviderStep) or in Settings.
 */
export function LinearScopeField({ value, onChange }: { value: ScopeRef | null; onChange: (s: ScopeRef | null) => void }) {
  const st = useProviderStatus("linear");
  const connected = st.connection === "connected";
  const scopes = useProviderScopes("linear", connected);
  const id = useId();

  if (!connected) {
    return (
      <span className="field-hint">
        {st.connection === "loading"
          ? "Checking Linear…"
          : st.status?.hasKey && st.error
            ? `Linear can't be reached: ${st.error}. You can add a source later.`
            : "Connect Linear in Settings → Integrations first. You can add a source later."}
      </span>
    );
  }
  return (
    <div className="field">
      <label className="field-label" htmlFor={id}>
        Team or project{st.viewer ? ` · connected as ${st.viewer}` : ""}
      </label>
      <ScopePicker
        id={id}
        scopes={scopes.data}
        loading={scopes.loading}
        error={scopes.error}
        value={value?.id ?? null}
        onChange={(sid) => onChange(scopes.data?.find((s) => s.id === sid) ?? null)}
      />
    </div>
  );
}
