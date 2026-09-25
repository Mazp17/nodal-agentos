import { useState } from "react";
import type { ProviderStatus } from "../../domain/api";
import type { ScopeRef } from "../../domain/types";
import { useProviderScopes, useProviderStatus } from "../../domain/hooks/providers";
import { KeyForm, ProviderMark } from "./parts";
import { ScopePicker } from "./ScopePicker";

export interface ConnectedProvider {
  provider: string;
  status: ProviderStatus;
  /**
   * Team or project chosen for the new project's source; `null` = connect the account
   * without a source. Whoever creates the project calls `createSourceLink` with this.
   */
  scope: ScopeRef | null;
}

export interface ConnectProviderStepProps {
  onConnected: (result: ConnectedProvider) => void;
  onSkip: () => void;
  onBack?: () => void;
  /** Label of the final button (onboarding ends here: "Create project"). */
  submitLabel?: string;
  /** Disables the buttons while the caller creates the project. */
  busy?: boolean;
}

/** Onboarding step 3: connect a task manager (optional). */
export function ConnectProviderStep({ onConnected, onSkip, onBack, submitLabel = "Continue", busy }: ConnectProviderStepProps) {
  const st = useProviderStatus("linear");
  const connected = st.connection === "connected" && st.status !== null;
  const scopes = useProviderScopes("linear", connected);
  const [scopeId, setScopeId] = useState<string | null>(null);
  const scope = scopes.data?.find((s) => s.id === scopeId) ?? null;

  return (
    <section className="pv-card pv-step" aria-labelledby="pv-step-title">
      <div className="pv-card-body pv-step-body">
        <header className="pv-step-head">
          <h2 id="pv-step-title" className="pv-step-title">
            Connect a task manager <span className="pv-muted">· optional</span>
          </h2>
          <p className="pv-page-sub">Import issues as tasks and keep their state in sync. Nodal works fully without one.</p>
        </header>

        <div className="pv-subcard">
          <div className="pv-card-row">
            <ProviderMark />
            <span className="pv-card-name">Linear</span>
            {connected && st.viewer && <span className="pv-conn pv-conn-ok">Connected as {st.viewer}</span>}
          </div>
          {st.connection === "loading" && <span className="pv-hint">Checking…</span>}
          {!connected && st.connection !== "loading" && (
            <>
              {st.status?.hasKey && st.error && (
                <div className="pv-alert pv-alert-danger" role="alert">
                  The saved key is not working: {st.error}
                </div>
              )}
              <KeyForm provider="linear" autoFocus />
            </>
          )}
          {connected && (
            <div className="pv-field">
              <span className="pv-field-label">Team or project</span>
              <div className="pv-field-body">
                <ScopePicker
                  scopes={scopes.data}
                  error={scopes.error}
                  loading={scopes.loading}
                  value={scopeId}
                  onChange={setScopeId}
                  allowNone="Don't add a source yet"
                />
              </div>
            </div>
          )}
        </div>

        <footer className="pv-step-foot">
          <span className="pv-hint">Asana, Azure DevOps · coming soon</span>
          <span className="pv-spacer" />
          {onBack && (
            <button type="button" className="btn btn-ghost" onClick={onBack} disabled={busy}>
              Back
            </button>
          )}
          <button type="button" className="btn" onClick={onSkip} disabled={busy}>
            Skip
          </button>
          {connected && st.status && (
            <button
              type="button"
              className="btn btn-primary"
              disabled={busy}
              onClick={() => st.status && onConnected({ provider: "linear", status: st.status, scope })}
            >
              {submitLabel}
            </button>
          )}
        </footer>
      </div>
    </section>
  );
}
