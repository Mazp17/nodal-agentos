// Elegir la carpeta de un repo: selector nativo de macOS, después `resolve_git_root` y los
// mensajes del diseño ("not inside a git repository", "Use repo root", "already added to…").

import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { basename } from "../../domain/hooks/projects";
import "./projects.css";

/** Raíz git canónica de la carpeta, o `null` si no está en un repo. */
export const resolveGitRoot = (path: string) => invoke<string | null>("resolve_git_root", { path });

/**
 * La raíz viene canónica (symlinks resueltos) y la carpeta elegida no: con el mismo
 * nombre final se toma como la raíz (p. ej. `/tmp/x` vs `/private/tmp/x`).
 */
const isSubfolder = (path: string, root: string) =>
  path.replace(/\/+$/, "") !== root.replace(/\/+$/, "") && basename(path) !== basename(root);

export type RepoNotice =
  | { kind: "bad"; text: string }
  | { kind: "sub"; text: string; root: string }
  | { kind: "dup"; text: string };

interface Options {
  /** Si el repo ya está en uso, dónde (`"Payments"`), o `""` si es un borrador sin nombre. */
  whereAdded: (root: string) => string | null;
  onPicked: (root: string) => void | Promise<void>;
}

/** Selector de repos con su aviso. `pick()` abre el diálogo nativo. */
export function useRepoPicker({ whereAdded, onPicked }: Options) {
  const [notice, setNotice] = useState<RepoNotice | null>(null);
  const [busy, setBusy] = useState(false);

  const accept = async (root: string) => {
    const where = whereAdded(root);
    if (where !== null) {
      setNotice({ kind: "dup", text: `${root} is already added${where ? ` to ${where}` : ""}.` });
      return;
    }
    setNotice(null);
    await onPicked(root);
  };

  const pick = async () => {
    setNotice(null);
    let picked: string | string[] | null;
    try {
      picked = await open({ directory: true, multiple: false, title: "Choose a repository folder" });
    } catch (e) {
      setNotice({ kind: "bad", text: `Couldn't open the folder picker: ${String(e)}` });
      return;
    }
    const path = typeof picked === "string" ? picked : null;
    if (!path) return;
    setBusy(true);
    try {
      const root = await resolveGitRoot(path);
      if (!root) {
        setNotice({ kind: "bad", text: `${path} is not inside a git repository. Pick the root of a git checkout.` });
      } else if (isSubfolder(path, root)) {
        setNotice({
          kind: "sub",
          root,
          text: `${basename(path)} is a subfolder of the git repo ${root}. Nodal works from the repo root.`,
        });
      } else {
        await accept(root);
      }
    } catch (e) {
      setNotice({ kind: "bad", text: String(e) });
    } finally {
      setBusy(false);
    }
  };

  const useRoot = async () => {
    if (notice?.kind !== "sub") return;
    setBusy(true);
    try {
      await accept(notice.root);
    } finally {
      setBusy(false);
    }
  };

  return { pick, notice, busy, useRoot, dismiss: () => setNotice(null), setNotice };
}

export function RepoNoticeBar({
  notice,
  onUseRoot,
  onDismiss,
}: {
  notice: RepoNotice;
  onUseRoot: () => void;
  onDismiss: () => void;
}) {
  return (
    <div className={`repo-notice repo-notice-${notice.kind}`} role={notice.kind === "sub" ? "status" : "alert"}>
      <span className="repo-notice-text">{notice.text}</span>
      {notice.kind === "sub" && (
        <button type="button" className="btn btn-primary btn-sm" onClick={onUseRoot}>
          Use repo root
        </button>
      )}
      <button type="button" className="icon-btn" aria-label="Dismiss" onClick={onDismiss}>
        ✕
      </button>
    </div>
  );
}

/** Fila de un repo en borrador (onboarding, New project). */
export function DraftRepoRow({ path, onRemove }: { path: string; onRemove: () => void }) {
  return (
    <div className="draft-repo">
      <span className="draft-repo-name">{basename(path)}</span>
      <span className="draft-repo-path mono ellipsis" title={path}>
        {path}
      </span>
      <button type="button" className="btn btn-ghost btn-xs draft-repo-remove" onClick={onRemove}>
        Remove
      </button>
    </div>
  );
}
