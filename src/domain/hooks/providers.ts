// Provider hooks (external sources): key status, links per project, scopes,
// states for the mapping and importables. The data lives in the backend; here there's only a
// shared cache for the key status (read by the sidebar, the pill and Settings at once)
// and a minimal bus so a mutation refreshes the other consumers.

import { useCallback, useEffect, useRef, useState, useSyncExternalStore } from "react";
import {
  listSourceLinks,
  providerListImportable,
  providerScopes,
  providerStatus,
  sourceRuleProjects,
  sourceStates,
  type ImportableItem,
  type ProviderStatus,
  type SourceStatesReport,
} from "../api";
import type { ScopeRef, SourceLink } from "../types";
import { registerResource } from "./store";

/** Commands reject with a string ready to display; anything else is normalized. */
export function errorText(err: unknown): string {
  if (typeof err === "string") return err;
  if (err && typeof err === "object" && "message" in err && typeof err.message === "string") return err.message;
  return String(err);
}

// ---------- Invalidation bus ----------

export type ProviderTopic = "links" | "status";

const topicSubs = new Map<ProviderTopic, Set<() => void>>();

function subscribeTopic(topic: ProviderTopic, fn: () => void): () => void {
  let set = topicSubs.get(topic);
  if (!set) topicSubs.set(topic, (set = new Set()));
  set.add(fn);
  return () => set.delete(fn);
}

/** Tells the subscribed hooks that `topic` changed (after a mutation). */
export function invalidateProviders(topic: ProviderTopic) {
  if (topic === "status") {
    for (const p of statusCache.keys()) void fetchStatus(p);
  }
  topicSubs.get(topic)?.forEach((fn) => fn());
}

// `nodal://changed` with `kind: "sources"` (sync, links) arrives as the "sources" resource.
registerResource("sources", async () => invalidateProviders("links"));

// ---------- Generic loading ----------

export interface Loaded<T> {
  data: T | null;
  error: string | null;
  loading: boolean;
  reload: () => void;
}

/**
 * Runs `load` when `key` changes (or when `topic` is invalidated). `key === null` doesn't load.
 * Discards stale responses if the key changed in the meantime.
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
    // Reloading the same key keeps the data (no flicker); with another key it doesn't.
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

// ---------- Provider status (shared) ----------

export type ProviderConnection = "loading" | "disconnected" | "connected" | "error";

interface StatusSnap {
  status: ProviderStatus | null;
  /** The command failed (not the key: that comes in `status.error`). */
  error: string | null;
  loading: boolean;
}

const EMPTY_SNAP: StatusSnap = { status: null, error: null, loading: true };
const statusCache = new Map<string, StatusSnap>();
const statusSubs = new Set<() => void>();
const statusInflight = new Map<string, Promise<void>>();
/** `provider_status` queries the provider: with the app open, it refreshes every 5 min. */
const STATUS_TTL_MS = 5 * 60_000;
const statusFetchedAt = new Map<string, number>();

function setSnap(provider: string, snap: StatusSnap) {
  statusCache.set(provider, snap);
  statusSubs.forEach((fn) => fn());
}

/** Bumped on every `setProviderStatus`: a read launched earlier no longer overwrites that value. */
const statusGen = new Map<string, number>();

function fetchStatus(provider: string): Promise<void> {
  const running = statusInflight.get(provider);
  if (running) return running;
  const prev = statusCache.get(provider) ?? EMPTY_SNAP;
  const gen = statusGen.get(provider) ?? 0;
  const stale = () => (statusGen.get(provider) ?? 0) !== gen;
  setSnap(provider, { ...prev, loading: true });
  const p = providerStatus(provider)
    .then(
      (status) => {
        if (!stale()) setSnap(provider, { status, error: null, loading: false });
      },
      (err) => {
        if (!stale()) setSnap(provider, { status: prev.status, error: errorText(err), loading: false });
      },
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

/** Sets the status without querying (e.g. with the response from `providerSetKey`). */
export function setProviderStatus(status: ProviderStatus) {
  statusGen.set(status.provider, (statusGen.get(status.provider) ?? 0) + 1);
  statusFetchedAt.set(status.provider, Date.now());
  setSnap(status.provider, { status, error: null, loading: false });
}

export interface ProviderStatusView {
  provider: string;
  status: ProviderStatus | null;
  /** Summary for rendering: no key, connected, or key present but invalid / offline. */
  connection: ProviderConnection;
  /** The user's name if the key is valid. */
  viewer: string | null;
  /** Key or command error, ready to display. */
  error: string | null;
  loading: boolean;
  refresh: () => Promise<void>;
}

/**
 * Key status of a provider (Linear by default), shared among everyone who uses
 * it: the sidebar, the global pill, Settings → Integrations and onboarding.
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

// ---------- Links, scopes, states, importables ----------

/** A project's sources (or all with `null`). Refreshes when `"links"` is invalidated. */
export function useSourceLinks(projectId: string | null): Loaded<SourceLink[]> {
  return useLoad(`links:${projectId ?? "*"}`, () => listSourceLinks(projectId), "links");
}

/** The provider's teams and projects. `enabled: false` doesn't query (no key). */
export function useProviderScopes(provider: string, enabled: boolean): Loaded<ScopeRef[]> {
  return useLoad(enabled ? `scopes:${provider}` : null, () => providerScopes(provider));
}

/** Provider project eligible as a rule, with the link it came from. */
export interface RuleProject {
  project: ScopeRef;
  linkId: string;
}

/**
 * Provider projects for project rules, merged across `linkIds` (no duplicates: the first
 * link wins). Hits the network; `enabled: false` or no links doesn't query.
 */
export function useRuleProjects(linkIds: string[], enabled = true): Loaded<RuleProject[]> {
  const key = enabled && linkIds.length ? `rule-projects:${linkIds.join(",")}` : null;
  return useLoad(key, async () => {
    const lists = await Promise.all(linkIds.map((id) => sourceRuleProjects(id)));
    const seen = new Set<string>();
    const out: RuleProject[] = [];
    lists.forEach((ps, i) => {
      for (const project of ps) {
        if (seen.has(project.id)) continue;
        seen.add(project.id);
        out.push({ project, linkId: linkIds[i] ?? "" });
      }
    });
    return out;
  });
}

/** Current states, mapping proposal and additions/removals against the known ones. Hits the network. */
export function useSourceStates(linkId: string | null): Loaded<SourceStatesReport> {
  return useLoad(linkId ? `states:${linkId}` : null, () => sourceStates(linkId ?? ""));
}

/** The link's importable items, filtered by `query` (already debounced). */
export function useImportable(linkId: string | null, query: string): Loaded<ImportableItem[]> {
  const q = query.trim();
  return useLoad(linkId ? `importable:${linkId}:${q}` : null, () =>
    providerListImportable(linkId ?? "", q ? q : null),
  );
}

/** `value` after `ms` without changes. */
export function useDebounced<T>(value: T, ms: number): T {
  const [v, setV] = useState(value);
  useEffect(() => {
    const id = window.setTimeout(() => setV(value), ms);
    return () => window.clearTimeout(id);
  }, [value, ms]);
  return v;
}

/** `Date.now()` that updates every `ms` (for "Synced 3m ago"). */
export function useNow(ms = 30_000): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const id = window.setInterval(() => setNow(Date.now()), ms);
    return () => window.clearInterval(id);
  }, [ms]);
  return now;
}
