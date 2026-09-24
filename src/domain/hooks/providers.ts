// Hooks de proveedores (fuentes externas): estado de la key, links por proyecto, scopes,
// estados para el mapeo e importables. Los datos viven en el backend; acá solo hay caché
// compartida para el estado de la key (lo leen el sidebar, la píldora y Settings a la vez)
// y un bus mínimo para que una mutación refresque a los demás consumidores.

import { useCallback, useEffect, useRef, useState, useSyncExternalStore } from "react";
import {
  listSourceLinks,
  providerListImportable,
  providerScopes,
  providerStatus,
  sourceStates,
  type ImportableItem,
  type ProviderStatus,
  type SourceStatesReport,
} from "../api";
import type { ScopeRef, SourceLink } from "../types";

/** Los comandos rechazan con un string listo para mostrar; cualquier otra cosa se normaliza. */
export function errorText(err: unknown): string {
  if (typeof err === "string") return err;
  if (err && typeof err === "object" && "message" in err && typeof err.message === "string") return err.message;
  return String(err);
}

// ---------- Bus de invalidación ----------

export type ProviderTopic = "links" | "status";

const topicSubs = new Map<ProviderTopic, Set<() => void>>();

function subscribeTopic(topic: ProviderTopic, fn: () => void): () => void {
  let set = topicSubs.get(topic);
  if (!set) topicSubs.set(topic, (set = new Set()));
  set.add(fn);
  return () => set.delete(fn);
}

/** Avisa a los hooks suscriptos que `topic` cambió (después de una mutación). */
export function invalidateProviders(topic: ProviderTopic) {
  if (topic === "status") {
    for (const p of statusCache.keys()) void fetchStatus(p);
  }
  topicSubs.get(topic)?.forEach((fn) => fn());
}

// ---------- Carga genérica ----------

export interface Loaded<T> {
  data: T | null;
  error: string | null;
  loading: boolean;
  reload: () => void;
}

/**
 * Corre `load` cuando cambia `key` (o al invalidar `topic`). `key === null` no carga.
 * Descarta respuestas viejas si la key cambió mientras tanto.
 */
function useLoad<T>(key: string | null, load: () => Promise<T>, topic?: ProviderTopic): Loaded<T> {
  const [state, setState] = useState<{ key: string | null; data: T | null; error: string | null; loading: boolean }>({
    key,
    data: null,
    error: null,
    loading: key !== null,
  });
  const [tick, setTick] = useState(0);
  const loadRef = useRef(load);
  loadRef.current = load;
  const reload = useCallback(() => setTick((n) => n + 1), []);

  useEffect(() => (topic ? subscribeTopic(topic, reload) : undefined), [topic, reload]);

  useEffect(() => {
    if (key === null) {
      setState({ key, data: null, error: null, loading: false });
      return;
    }
    let alive = true;
    // Al recargar la misma key se conservan los datos (sin parpadeo); con otra key no.
    setState((s) => (s.key === key ? { ...s, loading: true } : { key, data: null, error: null, loading: true }));
    loadRef.current().then(
      (data) => alive && setState({ key, data, error: null, loading: false }),
      (err) => alive && setState((s) => ({ key, data: s.key === key ? s.data : null, error: errorText(err), loading: false })),
    );
    return () => {
      alive = false;
    };
  }, [key, tick]);

  const fresh = state.key === key;
  return {
    data: fresh ? state.data : null,
    error: fresh ? state.error : null,
    loading: fresh ? state.loading : key !== null,
    reload,
  };
}

// ---------- Estado del proveedor (compartido) ----------

export type ProviderConnection = "loading" | "disconnected" | "connected" | "error";

interface StatusSnap {
  status: ProviderStatus | null;
  /** El comando falló (no la key: eso viene en `status.error`). */
  error: string | null;
  loading: boolean;
}

const EMPTY_SNAP: StatusSnap = { status: null, error: null, loading: true };
const statusCache = new Map<string, StatusSnap>();
const statusSubs = new Set<() => void>();
const statusInflight = new Map<string, Promise<void>>();
/** `provider_status` consulta al proveedor: con la app abierta, se refresca cada 5 min. */
const STATUS_TTL_MS = 5 * 60_000;
const statusFetchedAt = new Map<string, number>();

function setSnap(provider: string, snap: StatusSnap) {
  statusCache.set(provider, snap);
  statusSubs.forEach((fn) => fn());
}

function fetchStatus(provider: string): Promise<void> {
  const running = statusInflight.get(provider);
  if (running) return running;
  const prev = statusCache.get(provider) ?? EMPTY_SNAP;
  setSnap(provider, { ...prev, loading: true });
  const p = providerStatus(provider)
    .then(
      (status) => setSnap(provider, { status, error: null, loading: false }),
      (err) => setSnap(provider, { status: prev.status, error: errorText(err), loading: false }),
    )
    .finally(() => {
      statusInflight.delete(provider);
      statusFetchedAt.set(provider, Date.now());
    });
  statusInflight.set(provider, p);
  return p;
}

function subscribeStatus(fn: () => void) {
  statusSubs.add(fn);
  return () => {
    statusSubs.delete(fn);
  };
}

/** Fija el estado sin consultar (p. ej. con la respuesta de `providerSetKey`). */
export function setProviderStatus(status: ProviderStatus) {
  statusFetchedAt.set(status.provider, Date.now());
  setSnap(status.provider, { status, error: null, loading: false });
}

export interface ProviderStatusView {
  provider: string;
  status: ProviderStatus | null;
  /** Resumen para pintar: sin key, conectado, o key presente pero inválida / sin red. */
  connection: ProviderConnection;
  /** Nombre del usuario si la key es válida. */
  viewer: string | null;
  /** Error de la key o del comando, listo para mostrar. */
  error: string | null;
  loading: boolean;
  refresh: () => Promise<void>;
}

/**
 * Estado de la key de un proveedor (por defecto Linear), compartido entre todos los que lo
 * usan: el sidebar, la píldora global, Settings → Integrations y el onboarding.
 */
export function useProviderStatus(provider = "linear"): ProviderStatusView {
  const snap = useSyncExternalStore(subscribeStatus, () => statusCache.get(provider) ?? EMPTY_SNAP);

  useEffect(() => {
    const stale = () => Date.now() - (statusFetchedAt.get(provider) ?? 0) > STATUS_TTL_MS;
    if (!statusCache.has(provider) || stale()) void fetchStatus(provider);
    const id = window.setInterval(() => stale() && void fetchStatus(provider), 60_000);
    return () => window.clearInterval(id);
  }, [provider]);

  const refresh = useCallback(() => fetchStatus(provider), [provider]);
  const { status } = snap;
  const connection: ProviderConnection =
    !status && (snap.loading || !snap.error)
      ? "loading"
      : !status
        ? "error"
        : !status.hasKey
          ? "disconnected"
          : status.error
            ? "error"
            : "connected";

  return {
    provider,
    status,
    connection,
    viewer: status?.viewer ?? null,
    error: status?.error ?? snap.error,
    loading: snap.loading,
    refresh,
  };
}

// ---------- Links, scopes, estados, importables ----------

/** Fuentes de un proyecto (o de todos con `null`). Se refresca al invalidar `"links"`. */
export function useSourceLinks(projectId: string | null): Loaded<SourceLink[]> {
  return useLoad(`links:${projectId ?? "*"}`, () => listSourceLinks(projectId), "links");
}

/** Teams y proyectos del proveedor. `enabled: false` no consulta (sin key). */
export function useProviderScopes(provider: string, enabled: boolean): Loaded<ScopeRef[]> {
  return useLoad(enabled ? `scopes:${provider}` : null, () => providerScopes(provider));
}

/** Estados actuales, propuesta de mapeo y altas/bajas contra lo conocido. Va a la red. */
export function useSourceStates(linkId: string | null): Loaded<SourceStatesReport> {
  return useLoad(linkId ? `states:${linkId}` : null, () => sourceStates(linkId ?? ""));
}

/** Ítems importables del link, filtrados por `query` (ya con debounce). */
export function useImportable(linkId: string | null, query: string): Loaded<ImportableItem[]> {
  const q = query.trim();
  return useLoad(linkId ? `importable:${linkId}:${q}` : null, () =>
    providerListImportable(linkId ?? "", q ? q : null),
  );
}

/** `value` después de `ms` sin cambios. */
export function useDebounced<T>(value: T, ms: number): T {
  const [v, setV] = useState(value);
  useEffect(() => {
    const id = window.setTimeout(() => setV(value), ms);
    return () => window.clearTimeout(id);
  }, [value, ms]);
  return v;
}

/** `Date.now()` que se actualiza cada `ms` (para "Synced 3m ago"). */
export function useNow(ms = 30_000): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const id = window.setInterval(() => setNow(Date.now()), ms);
    return () => window.clearInterval(id);
  }, [ms]);
  return now;
}
