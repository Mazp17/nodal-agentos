import { useEffect, useRef } from "react";
import { formatTokens } from "../../lib/format";
import { contextPercent, DEFAULT_CONTEXT_WINDOW, optionLabel, type PillOption } from "./model";

export interface PillProps {
  id: string;
  /** Small label before the value (`Mode`, `Repo`). */
  caption?: string;
  label: string;
  options: PillOption[];
  value: string | null;
  open: boolean;
  /** Right-hand pills open their menu aligned to the right edge. */
  align: "left" | "right";
  tone?: "warn";
  disabled?: boolean;
  onToggle: (id: string | null) => void;
  onPick: (value: string | null) => void;
}

export function Pill(p: PillProps) {
  const menuId = `chat-pill-${p.id}`;
  return (
    <div className="chat-pill-wrap">
      <button
        type="button"
        className={`chat-pill ${p.caption ? "chat-pill-boxed" : ""} ${p.open ? "on" : ""} ${p.tone === "warn" ? "chat-pill-warn" : ""}`}
        aria-haspopup="menu"
        aria-expanded={p.open}
        aria-controls={p.open ? menuId : undefined}
        aria-label={p.caption ? `${p.caption}: ${p.label}` : p.label}
        disabled={p.disabled}
        onClick={() => p.onToggle(p.open ? null : p.id)}
      >
        {p.caption && <span className="chat-pill-caption">{p.caption}</span>}
        <span className="ellipsis">{p.label}</span>
        <span className="chat-pill-chev" aria-hidden>
          ▾
        </span>
      </button>
      {p.open && (
        <>
          <div className="chat-menu-scrim" onClick={() => p.onToggle(null)} aria-hidden />
          <div id={menuId} role="menu" className={`chat-menu chat-menu-${p.align}`}>
            {p.options.map((o) => {
              const on = o.value === p.value;
              return (
                <button
                  key={o.value ?? "default"}
                  type="button"
                  role="menuitemradio"
                  aria-checked={on}
                  className="chat-menu-item"
                  onClick={() => {
                    p.onToggle(null);
                    if (!on) p.onPick(o.value);
                  }}
                >
                  <span className="chat-menu-mark" aria-hidden>
                    {on ? "✓" : ""}
                  </span>
                  <span className="chat-menu-text">
                    <span>{o.label}</span>
                    {o.hint && <span className="chat-menu-hint">{o.hint}</span>}
                  </span>
                </button>
              );
            })}
          </div>
        </>
      )}
    </div>
  );
}

export function ContextRing({ tokens, window }: { tokens: number | null; window: number | null }) {
  const pct = contextPercent(tokens, window);
  const title =
    pct == null
      ? "Context window used: shown after the first answer"
      : `Context window used: ${formatTokens(tokens)} of ${formatTokens(window ?? DEFAULT_CONTEXT_WINDOW)} tokens`;
  return (
    <span className="chat-ctx num" title={title} role="img" aria-label={title}>
      <span className="chat-ctx-ring" style={{ ["--pct" as string]: `${pct ?? 0}` }} aria-hidden />
      {pct == null ? "—" : `${pct}%`}
    </span>
  );
}

export interface ComposerSettings {
  permissionMode: string | null;
  repoId: string | null;
  model: string | null;
  effort: string | null;
}

export interface ComposerProps {
  draft: string;
  placeholder: string;
  settings: ComposerSettings;
  modes: PillOption[];
  repoOptions: PillOption[];
  models: PillOption[];
  efforts: PillOption[];
  menu: string | null;
  busy: boolean;
  contextTokens: number | null;
  contextWindow: number | null;
  focusKey: string;
  onDraft: (text: string) => void;
  onMenu: (id: string | null) => void;
  onSetting: (key: keyof ComposerSettings, value: string | null) => void;
  onSend: () => void;
  onStop: () => void;
}

export function Composer(p: ComposerProps) {
  const ref = useRef<HTMLTextAreaElement>(null);
  useEffect(() => ref.current?.focus(), [p.focusKey]);

  const lines = p.draft.split("\n").length;
  const canSend = !!p.draft.trim() && !p.busy;
  const pill = (id: keyof ComposerSettings, options: PillOption[], align: "left" | "right", caption?: string) => ({
    id,
    caption,
    options,
    align,
    value: p.settings[id],
    label: optionLabel(options, p.settings[id]),
    open: p.menu === id,
    onToggle: p.onMenu,
    onPick: (v: string | null) => p.onSetting(id, v),
  });

  return (
    <div className="chat-composer-wrap">
      <div
        className="chat-composer"
        onKeyDown={(e) => {
          if (e.key === "Escape" && p.menu) {
            e.stopPropagation();
            p.onMenu(null);
          }
        }}
      >
        <textarea
          ref={ref}
          className="chat-input"
          value={p.draft}
          placeholder={p.placeholder}
          aria-label="Message"
          style={{ height: Math.min(200, Math.max(64, lines * 19 + 30)) }}
          onChange={(e) => p.onDraft(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
              e.preventDefault();
              if (canSend) p.onSend();
            }
          }}
        />
        <div className="chat-composer-bar">
          <Pill
            {...pill("permissionMode", p.modes, "left", "Mode")}
            tone={p.settings.permissionMode === "bypassPermissions" ? "warn" : undefined}
          />
          <Pill {...pill("repoId", p.repoOptions, "left", "Repo")} />
          <span className="spacer" />
          <ContextRing tokens={p.contextTokens} window={p.contextWindow} />
          <Pill {...pill("model", p.models, "right")} />
          <Pill {...pill("effort", p.efforts, "right")} />
          {p.busy ? (
            <button type="button" className="chat-send chat-stop" title="Stop" aria-label="Stop the answer" onClick={p.onStop}>
              ■
            </button>
          ) : (
            <button
              type="button"
              className={`chat-send ${canSend ? "on" : ""}`}
              title="Send (Enter)"
              aria-label="Send"
              disabled={!canSend}
              onClick={p.onSend}
            >
              ↑
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
