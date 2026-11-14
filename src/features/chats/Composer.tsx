import { useEffect, useRef, type ReactNode } from "react";
import { ArrowUp, Check, ChevronDown, Folder, Gauge, Square, Star, User, Zap, type LucideIcon } from "lucide-react";
import { formatTokens } from "../../lib/format";
import { contextPercent, DEFAULT_CONTEXT_WINDOW, optionLabel, type PillOption } from "./model";

export type PillIcon = "mode" | "repo" | "agent" | "model" | "effort";
const ICONS: Record<PillIcon, LucideIcon> = { mode: Zap, repo: Folder, agent: User, model: Star, effort: Gauge };

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
  const Icon = p.icon && ICONS[p.icon];
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
        {Icon && <Icon className="chat-pill-icon" size={13} strokeWidth={1.75} aria-hidden />}
        <span className="ellipsis">{p.label}</span>
        {!p.disabled && p.joined !== "start" && (
          <ChevronDown className="chat-pill-chev" size={10} strokeWidth={2} aria-hidden />
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
                    {on && <Check size={12} strokeWidth={2} aria-hidden />}
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
  /** Attached strip above the textarea (the changes bar). */
  tray?: ReactNode;
}

export function Composer(p: ComposerProps) {
  const ref = useRef<HTMLTextAreaElement>(null);
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    el.focus();
    // A restored draft would otherwise get the caret at its start (WebKit).
    el.setSelectionRange(el.value.length, el.value.length);
  }, [p.focusKey]);

  const lines = p.draft.split("\n").length;
  const canSend = !!p.draft.trim();
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
        {p.tray && <div className="chat-composer-tray">{p.tray}</div>}
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
          {p.busy && (
            <button type="button" className="chat-send chat-stop" title="Stop" aria-label="Stop the answer" onClick={p.onStop}>
              <Square size={10} fill="currentColor" aria-hidden />
            </button>
          )}
          {(!p.busy || canSend) && (
            <button
              type="button"
              className={`chat-send ${canSend ? "on" : ""}`}
              title={p.busy ? "Queue (Enter)" : "Send (Enter)"}
              aria-label={p.busy ? "Queue the message" : "Send"}
              disabled={!canSend}
              onClick={p.onSend}
            >
              <ArrowUp size={14} strokeWidth={2} aria-hidden />
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
