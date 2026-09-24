// Estado de Linear (único proveedor por ahora) para el pie del sidebar, el onboarding y el
// diálogo de proyecto. `provider_status` valida la key contra la API: se consulta poco.

import { createContext, useCallback, useContext, useEffect, useRef, useState, type ReactNode } from "react";
import { providerStatus, type ProviderStatus } from "../domain/api";

export const LINEAR = "linear";

interface State {
  /** `null` mientras no hay respuesta. */
  linear: ProviderStatus | null;
  /** Error al consultar (no el de la key, que viene en `linear.error`). */
  error: string | null;
  refresh: () => Promise<void>;
  /** Tras guardar o borrar la key desde la UI. */
  set: (s: ProviderStatus) => void;
}

const Ctx = createContext<State | null>(null);
const POLL_MS = 120_000;

export function ProviderStatusProvider({ children }: { children: ReactNode }) {
  const [linear, setLinear] = useState<ProviderStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const seq = useRef(0);

  const refresh = useCallback(async () => {
    const id = ++seq.current;
    try {
      const s = await providerStatus(LINEAR);
      if (id !== seq.current) return;
      setLinear(s);
      setError(null);
    } catch (e) {
      if (id === seq.current) setError(String(e));
    }
  }, []);

  const set = useCallback((s: ProviderStatus) => {
    seq.current++;
    setLinear(s);
    setError(null);
  }, []);

  useEffect(() => {
    void refresh();
    const t = setInterval(() => void refresh(), POLL_MS);
    return () => clearInterval(t);
  }, [refresh]);

  return <Ctx.Provider value={{ linear, error, refresh, set }}>{children}</Ctx.Provider>;
}

export function useProviderStatus(): State {
  const v = useContext(Ctx);
  if (!v) throw new Error("useProviderStatus must be used inside <ProviderStatusProvider>");
  return v;
}

/** Conectado = key guardada y válida. */
export const isConnected = (s: ProviderStatus | null) => !!s?.hasKey && !s.error;
