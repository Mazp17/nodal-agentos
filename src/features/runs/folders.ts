import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { homeDir } from "@tauri-apps/api/path";
import { resolveGitRoot } from "./api";

let homeCache: Promise<string | null> | null = null;

function home(): Promise<string | null> {
  homeCache ??= homeDir()
    .then((h) => h.replace(/\/+$/, ""))
    .catch(() => null);
  return homeCache;
}

/** Home del usuario (sin barra final), para abreviar rutas con `~`. */
export function useHome(): string | null {
  const [h, setH] = useState<string | null>(null);
  useEffect(() => {
    let alive = true;
    void home().then((v) => alive && setH(v));
    return () => {
      alive = false;
    };
  }, []);
  return h;
}

/** `/Users/x/Code/repo` → `~/Code/repo`. */
export function tildify(path: string, homePath: string | null): string {
  if (!homePath) return path;
  if (path === homePath) return "~";
  return path.startsWith(`${homePath}/`) ? `~${path.slice(homePath.length)}` : path;
}

export type PickResult = { path: string; isRepo: boolean } | null;

/**
 * Selector nativo de carpetas. Si eligen una subcarpeta de un repo, ofrece usar la raíz.
 * `null` si cancelan. `isRepo` = la ruta devuelta es la raíz de un repo git.
 */
export async function pickRepoFolder(current: string | null | undefined): Promise<PickResult> {
  const h = await home();
  const defaultPath = current?.trim() || (h ? `${h}/Code` : undefined);
  const picked = await open({ directory: true, multiple: false, defaultPath, title: "Choose a git repository" });
  if (typeof picked !== "string") return null;
  const chosen = picked.replace(/\/+$/, "") || "/";
  let root: string | null;
  try {
    root = await resolveGitRoot(chosen);
  } catch {
    root = null;
  }
  if (root === null) return { path: chosen, isRepo: false };
  if (root === chosen) return { path: chosen, isRepo: true };
  const useRoot = window.confirm(
    `${tildify(chosen, h)} is inside the git repo ${tildify(root, h)}.\n\nUse the repo root instead?`,
  );
  return useRoot ? { path: root, isRepo: true } : { path: chosen, isRepo: false };
}
