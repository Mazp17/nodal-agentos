import { useEffect, useRef, type ReactNode } from "react";
import { formatTokens } from "../../lib/format";
import { contextPercent, DEFAULT_CONTEXT_WINDOW, optionLabel, type PillOption } from "./model";

/** Field glyphs, drawn on the same 14px grid and stroke as the app's other icons. */
export type PillIcon = "mode" | "repo" | "agent" | "model" | "effort";
const ICONS: Record<PillIcon, ReactNode> = {
  mode: <path d="M7 1.6l4.6 1.8v3.3c0 2.9-2 4.9-4.6 5.7-2.6-.8-4.6-2.8-4.6-5.7V3.4z" />,
  repo: <path d="M1.8 3.6a1 1 0 0 1 1-1h2.8l1.2 1.4h4.4a1 1 0 0 1 1 1v5.4a1 1 0 0 1-1 1H2.8a1 1 0 0 1-1-1z" />,
  agent: (
    <>
      <circle cx="7" cy="7" r="2.2" />
      <path d="M9.2 7v.9a1.6 1.6 0 0 0 3.2 0V7A5.4 5.4 0 1 0 10 11.5" />
    </>
  ),
  model: <path d="M7 1.8l1.2 3.3 3.3 1.2-3.3 1.2L7 10.8 5.8 7.5 2.5 6.3l3.3-1.2zM11 9.6l.5 1.3 1.3.5-1.3.5-.5 1.3-.5-1.3-1.3-.5 1.3-.5z" />,
  effort: <path d="M2.5 11.5V8.5M5.5 11.5V6.5M8.5 11.5V4.5M11.5 11.5V2.5" />,
};

export interface PillProps {
  id: string;
  icon?: PillIcon;
  /** Joined to its neighbour in a `.chat-pill-group` (model · effort). */
  joined?: "start" | "end";
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
  /** Tooltip, e.g. why the pill is disabled. */
  title?: string;
  onToggle: (id: string | null) => void;
  onPick: (value: string | null) => void;
}

export function Pill(p: PillProps) {
  const menuId = `chat-pill-${p.id}`;
  return (
    // The tooltip sits on the wrapper: a disabled button gets no hover events.
    <div className="chat-pill-wrap" title={p.title}>
      <button
        type="button"
        className={`chat-pill ${p.open ? "on" : ""} ${p.tone === "warn" ? "chat-pill-warn" : ""} ${p.joined ? `chat-pill-${p.joined}` : ""}`}
        title={p.caption ? `${p.caption}: ${p.label}` : undefined}
        aria-haspopup="menu"
        aria-expanded={p.open}
        aria-controls={p.open ? menuId : undefined}
        aria-label={p.caption ? `${p.caption}: ${p.label}` : p.label}
        disabled={p.disabled}
        onClick={() => p.onToggle(p.open ? null : p.id)}
      >
        {p.icon && (
          <svg className="chat-pill-icon" width="13" height="13" viewBox="0 0 14 14" fill="none" stroke="currentColor" strokeWidth="1.3" strokeLinecap="round" strokeLinejoin="round" aria-hidden focusable="false">
            {ICONS[p.icon]}
          </svg>
        )}
        <span className="ellipsis">{p.label}</span>
        {!p.joined && (
          <svg className="chat-pill-chev" width="9" height="9" viewBox="0 0 10 10" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" aria-hidden focusable="false">
            <path d="M2.5 4l2.5 2.5L7.5 4" />
          </svg>
        )}
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
                    {on && (
                      <svg width="11" height="11" viewBox="0 0 12 12" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" focusable="false">
                        <path d="M2.5 6.2l2.3 2.3 4.7-5" />
                      </svg>
                    )}
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
  agent: string | null;
  model: string | null;
  effort: string | null;
}

export interface ComposerProps {
  draft: string;
  placeholder: string;
  settings: ComposerSettings;
  modes: PillOption[];
  repoOptions: PillOption[];
  agents: PillOption[];
  /** The chat already started: Claude Code keeps a session's agent. */
  agentLocked: boolean;
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
  const pill = (id: keyof ComposerSettings, options: PillOption[], align: "left" | "right", caption: string, icon: PillIcon) => ({
    id,
    caption,
    icon,
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
            {...pill("agent", p.agents, "left", "Agent", "agent")}
            disabled={p.agentLocked}
            title={p.agentLocked ? "A chat keeps the agent it started with. Start a new chat to talk to another one." : undefined}
          />
          <Pill {...pill("repoId", p.repoOptions, "left", "Repo", "repo")} />
          <Pill
            {...pill("permissionMode", p.modes, "left", "Mode", "mode")}
            tone={p.settings.permissionMode === "auto" || p.settings.permissionMode === "bypassPermissions" ? "warn" : undefined}
          />
          <span className="spacer" />
          <ContextRing tokens={p.contextTokens} window={p.contextWindow} />
          {/* Model and effort read as one setting, "Opus · high", each half with its own menu. */}
          <span className="chat-pill-group" role="group" aria-label="Model and effort">
            <Pill {...pill("model", p.models, "right", "Model", "model")} joined="start" />
            <Pill {...pill("effort", p.efforts, "right", "Effort", "effort")} joined="end" />
          </span>
          {p.busy ? (
            <button type="button" className="chat-send chat-stop" title="Stop" aria-label="Stop the answer" onClick={p.onStop}>
              <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden focusable="false">
                <rect x="1" y="1" width="8" height="8" rx="1.5" fill="currentColor" />
              </svg>
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
              <svg width="13" height="13" viewBox="0 0 12 12" fill="none" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round" aria-hidden focusable="false">
                <path d="M6 10V2M2.5 5.5L6 2l3.5 3.5" />
              </svg>
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
