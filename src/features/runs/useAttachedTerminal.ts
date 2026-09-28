// xterm.js bound to a `claude attach` PTY (see `pty_*` in api.ts). The TUI draws everything
// (transcript, permission prompts, input); this only moves bytes and keeps the PTY sized.

import { useCallback, useEffect, useLayoutEffect, useRef, useState, type RefObject } from "react";
import { Channel } from "@tauri-apps/api/core";
import { Terminal, type IDisposable, type ITheme } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { WebglAddon } from "@xterm/addon-webgl";
import { ptyAttach, ptyClose, ptyResize, ptyWrite } from "./api";

export type AttachStatus = "connecting" | "attached" | "ended" | "error";

export interface AttachedTerminal {
  /** Element xterm renders into (no padding: FitAddon measures its parent). */
  hostRef: RefObject<HTMLDivElement | null>;
  status: AttachStatus;
  error: string | null;
  /** Exit code of `claude attach` once `ended` (`null` if unknown). */
  exitCode: number | null;
  cols: number | null;
  rows: number | null;
  /** Starts a new attach (new terminal) after it ended or failed. */
  reattach: () => void;
}

const FONT_SIZE = 13;

// ---- Theme: tokens are oklch/rgb; xterm wants plain colors, so they're resolved on a canvas. ----

let probe: CanvasRenderingContext2D | null = null;

/** A CSS color resolved to `[r, g, b, a]` (a in 0–1), or `null` if the webview can't parse it. */
function resolveColor(css: string): [number, number, number, number] | null {
  if (!css) return null;
  probe ??= document.createElement("canvas").getContext("2d", { willReadFrequently: true });
  if (!probe) return null;
  // Invalid colors leave `fillStyle` untouched: a sentinel detects that.
  probe.fillStyle = "#010203";
  probe.fillStyle = css;
  if (probe.fillStyle === "#010203" && css.toLowerCase() !== "#010203") return null;
  probe.clearRect(0, 0, 1, 1);
  probe.fillRect(0, 0, 1, 1);
  const [r = 0, g = 0, b = 0, a = 255] = probe.getImageData(0, 0, 1, 1).data;
  return [r, g, b, a / 255];
}

const hex = (n: number) => n.toString(16).padStart(2, "0");

function tokenColor(style: CSSStyleDeclaration, name: string, alpha?: number): string | undefined {
  const c = resolveColor(style.getPropertyValue(name).trim());
  if (!c) return undefined;
  const [r, g, b] = c;
  return alpha === undefined ? `#${hex(r)}${hex(g)}${hex(b)}` : `rgba(${r}, ${g}, ${b}, ${alpha})`;
}

/** Nodal's palette mapped onto xterm; any token that doesn't resolve keeps xterm's default. */
function readTheme(): ITheme {
  const s = getComputedStyle(document.documentElement);
  const t = (name: string, alpha?: number) => tokenColor(s, name, alpha);
  const theme: ITheme = {
    background: t("--bg-deep"),
    foreground: t("--text-2"),
    cursor: t("--accent"),
    cursorAccent: t("--bg-deep"),
    selectionBackground: t("--accent", 0.3),
    selectionInactiveBackground: t("--accent", 0.15),
    black: t("--surface-selected"),
    red: t("--red"),
    green: t("--green"),
    yellow: t("--amber"),
    blue: t("--info"),
    magenta: t("--project-4"),
    cyan: t("--project-7"),
    white: t("--text-4"),
    brightBlack: t("--gray"),
    brightRed: t("--red-text"),
    brightGreen: t("--green"),
    brightYellow: t("--amber-text"),
    brightBlue: t("--project-2"),
    brightMagenta: t("--project-8"),
    brightCyan: t("--project-7"),
    brightWhite: t("--text-strong"),
  };
  // Drop unresolved entries so xterm falls back to its own defaults.
  return Object.fromEntries(Object.entries(theme).filter(([, v]) => v !== undefined)) as ITheme;
}

const encoder = new TextEncoder();

/** `onBinary` strings carry one byte per char (mouse reports in X10 mode). */
function binaryBytes(s: string): Uint8Array {
  const out = new Uint8Array(s.length);
  for (let i = 0; i < s.length; i++) out[i] = s.charCodeAt(i) & 0xff;
  return out;
}

function concat(chunks: Uint8Array[]): Uint8Array {
  if (chunks.length === 1) return chunks[0]!;
  const out = new Uint8Array(chunks.reduce((n, c) => n + c.length, 0));
  let at = 0;
  for (const c of chunks) {
    out.set(c, at);
    at += c.length;
  }
  return out;
}

/** Shift+Esc: the keyboard way out of the terminal (plain Esc belongs to Claude). */
const isCloseKey = (e: KeyboardEvent) => e.key === "Escape" && e.shiftKey && !e.metaKey && !e.ctrlKey && !e.altKey;

interface AttachState {
  status: AttachStatus;
  error: string | null;
  exitCode: number | null;
}

const CONNECTING: AttachState = { status: "connecting", error: null, exitCode: null };

/**
 * Attaches to the background session `claudeId` in an embedded terminal. Unmounting (or
 * changing the id) detaches; the run keeps going. `onRequestClose` runs on Shift+Esc
 * inside the terminal.
 */
export function useAttachedTerminal(claudeId: string, cwd: string | null, onRequestClose?: () => void): AttachedTerminal {
  const hostRef = useRef<HTMLDivElement>(null);
  const closeRef = useRef(onRequestClose);
  useLayoutEffect(() => {
    closeRef.current = onRequestClose;
  });
  const [attempt, setAttempt] = useState(0);
  const [state, setState] = useState<AttachState>(CONNECTING);
  const [size, setSize] = useState<{ cols: number; rows: number } | null>(null);

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;
    // StrictMode mounts twice: anything that resolves after cleanup must undo itself.
    let disposed = false;
    let exited = false;
    let session: number | null = null;
    let term: Terminal | null = null;
    let observer: ResizeObserver | null = null;
    let frame = 0;
    const subs: IDisposable[] = [];

    // `pty_write` is async on the Rust side: concurrent invokes could reorder keystrokes.
    // One write in flight at a time; whatever arrives meanwhile goes out as a single write.
    let pending: Uint8Array[] = [];
    let writing = false;
    const flush = async () => {
      if (writing) return;
      writing = true;
      while (pending.length > 0 && session !== null && !exited && !disposed) {
        const chunk = concat(pending);
        pending = [];
        try {
          await ptyWrite(session, chunk);
        } catch (e) {
          // The process may be exiting (`onExit` reports it); keep the queue going.
          console.warn("pty_write", e);
        }
      }
      if (exited || disposed) pending = [];
      writing = false;
    };
    const send = (bytes: Uint8Array) => {
      if (session === null || exited || bytes.length === 0) return;
      pending.push(bytes);
      void flush();
    };

    const start = async () => {
      try {
        // xterm measures the cell once at open: the font must be ready or the grid is off.
        await document.fonts.load(`${FONT_SIZE}px 'JetBrains Mono'`);
      } catch {
        /* falls back to the next mono font */
      }
      if (disposed) return;

      const t = new Terminal({
        fontFamily: "'JetBrains Mono', ui-monospace, 'SF Mono', Menlo, monospace",
        fontSize: FONT_SIZE,
        // Tighter than the UI's line height: box-drawing glyphs must touch between rows.
        lineHeight: 1.1,
        cursorBlink: true,
        // Option composes characters (accents, `|`, `@` on some layouts) as on macOS.
        macOptionIsMeta: false,
        // The TUI enables mouse tracking; Option+drag still selects text.
        macOptionClickForcesSelection: true,
        scrollback: 1000,
        theme: readTheme(),
      });
      term = t;
      const fit = new FitAddon();
      t.loadAddon(fit);
      t.open(host);
      // Before attaching: Shift+Esc must work while connecting and after the session ends.
      t.attachCustomKeyEventHandler((e) => {
        if (!isCloseKey(e)) return true;
        if (e.type === "keydown") {
          // Handled here: AppShell's Esc cascade must not also close the task panel.
          e.preventDefault();
          e.stopPropagation();
          closeRef.current?.();
        }
        return false;
      });
      try {
        const gl = new WebglAddon();
        gl.onContextLoss(() => gl.dispose());
        t.loadAddon(gl);
      } catch {
        /* no WebGL: xterm's DOM renderer is used */
      }
      fit.fit();
      let sent = { cols: t.cols, rows: t.rows };
      setSize(sent);

      observer = new ResizeObserver(() => {
        cancelAnimationFrame(frame);
        frame = requestAnimationFrame(() => {
          if (disposed) return;
          fit.fit();
          if (t.cols === sent.cols && t.rows === sent.rows) return;
          sent = { cols: t.cols, rows: t.rows };
          setSize(sent);
          // A dead session (exited, or never attached) gets no resize: the footer still updates.
          if (session !== null && !exited) {
            ptyResize(session, sent.cols, sent.rows).catch(() => {
              /* the process may be exiting */
            });
          }
        });
      });
      observer.observe(host);

      const onData = new Channel<ArrayBuffer>();
      onData.onmessage = (buf) => {
        if (!disposed) t.write(new Uint8Array(buf));
      };
      const onExit = new Channel<number | null>();
      onExit.onmessage = (code) => {
        exited = true;
        if (!disposed) setState({ status: "ended", error: null, exitCode: code });
      };

      let id: number;
      try {
        id = await ptyAttach(claudeId, cwd, sent.cols, sent.rows, onData, onExit);
      } catch (e) {
        if (!disposed) setState({ status: "error", error: String(e), exitCode: null });
        return;
      }
      if (disposed) {
        void ptyClose(id).catch(() => {});
        return;
      }
      session = id;
      if (exited) return;
      // The drawer may have resized while attaching.
      if (t.cols !== sent.cols || t.rows !== sent.rows) {
        sent = { cols: t.cols, rows: t.rows };
        setSize(sent);
        void ptyResize(id, sent.cols, sent.rows).catch(() => {});
      }
      subs.push(
        t.onData((d) => send(encoder.encode(d))),
        t.onBinary((d) => send(binaryBytes(d))),
      );
      setState({ status: "attached", error: null, exitCode: null });
      t.focus();
    };
    void start();

    return () => {
      disposed = true;
      cancelAnimationFrame(frame);
      observer?.disconnect();
      for (const s of subs) s.dispose();
      if (session !== null) void ptyClose(session).catch(() => {});
      term?.dispose();
    };
  }, [claudeId, cwd, attempt]);

  const reattach = useCallback(() => {
    setState(CONNECTING);
    setAttempt((a) => a + 1);
  }, []);

  return { hostRef, status: state.status, error: state.error, exitCode: state.exitCode, cols: size?.cols ?? null, rows: size?.rows ?? null, reattach };
}
