import { useState } from "react";
import { providerClearKey } from "../../domain/api";
import {
  errorText,
  invalidateProviders,
  setProviderStatus,
  useProviderStatus,
  useSourceLinks,
} from "../../domain/hooks/providers";
import { useToast } from "../../ui/Toasts";
import { InlineConfirm, KeyForm, ProviderMark } from "./parts";
import { PROVIDERS, plural } from "./meta";

/** Settings → Integrations: key de Linear y proveedores por venir. */
export function IntegrationsSettings() {
  return (
    <section className="pv-page" aria-labelledby="pv-int-title">
      <header className="pv-page-head">
        <h2 id="pv-int-title" className="pv-page-title">
          Integrations
        </h2>
        <p className="pv-page-sub">Task managers are optional sources. Nodal works fully without one.</p>
      </header>
      <LinearCard />
      {PROVIDERS.filter((p) => !p.available).map((p) => (
        <div key={p.id} className="pv-card pv-card-soon">
          <div className="pv-card-row">
            <ProviderMark />
            <span className="pv-card-name">{p.name}</span>
            <span className="pv-pill">Coming soon</span>
          </div>
        </div>
      ))}
    </section>
  );
}

function LinearCard() {
  const st = useProviderStatus("linear");
  const links = useSourceLinks(null);
  const toast = useToast();
  const [editing, setEditing] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [deleting, setDeleting] = useState(false);

  const hasKey = st.status?.hasKey ?? false;
  const linearLinks = (links.data ?? []).filter((l) => l.provider === "linear");
  const projects = new Set(linearLinks.map((l) => l.projectId)).size;

  const remove = async () => {
    setDeleting(true);
    try {
      setProviderStatus(await providerClearKey("linear"));
      invalidateProviders("links");
      setConfirmDelete(false);
      setEditing(false);
      toast("Linear key deleted", "Sources are paused. Nodal keeps working with local tasks.", "warn");
    } catch (err) {
      toast("Could not delete the key", errorText(err), "danger");
    } finally {
      setDeleting(false);
    }
  };

  return (
    <div className="pv-card">
      <div className="pv-card-body">
        <div className="pv-card-row">
          <ProviderMark />
          <span className="pv-card-name">Linear</span>
          <ConnectionLabel st={st} />
        </div>

        {hasKey && (
          <>
            <div className="pv-keybox">
              <span className="mono pv-keymask" aria-label="Stored API key, hidden">
                lin_api_••••••••••••••••
              </span>
              <span className="pv-hint">macOS Keychain</span>
            </div>
            {st.connection === "error" && st.error && (
              <div className="pv-alert pv-alert-danger" role="alert">
                {st.error}
              </div>
            )}
            <span className="pv-hint">
              {links.data
                ? `${plural(linearLinks.length, "source")} across ${plural(projects, "project")}`
                : links.error ?? "Loading sources…"}
            </span>
          </>
        )}
        {!hasKey && st.connection === "error" && st.error && (
          <div className="pv-alert pv-alert-danger" role="alert">
            <span className="pv-alert-text">Could not read the key status. {st.error}</span>
            <button type="button" className="btn btn-sm" onClick={() => void st.refresh()}>
              Retry
            </button>
          </div>
        )}

        {editing && (
          <KeyForm
            provider="linear"
            autoFocus
            onSaved={(s) => {
              setEditing(false);
              invalidateProviders("links");
              toast("Linear connected", s.viewer ? `Signed in as ${s.viewer}.` : undefined, "ok");
            }}
          />
        )}

        {confirmDelete ? (
          <InlineConfirm
            text="Delete the Linear key? Sources pause until you add a key again. Imported tasks stay."
            confirmLabel="Delete key"
            busy={deleting}
            onConfirm={() => void remove()}
            onCancel={() => setConfirmDelete(false)}
          />
        ) : (
          <div className="pv-actions">
            {hasKey ? (
              <>
                <button type="button" className="btn btn-sm" onClick={() => setEditing((v) => !v)}>
                  {editing ? "Cancel" : "Replace key"}
                </button>
                <button type="button" className="btn btn-sm btn-danger" onClick={() => setConfirmDelete(true)}>
                  Delete key
                </button>
              </>
            ) : (
              st.connection !== "loading" && (
                <button
                  type="button"
                  className={`btn btn-sm${editing ? "" : " btn-primary"}`}
                  onClick={() => setEditing((v) => !v)}
                >
                  {editing ? "Cancel" : "Add key"}
                </button>
              )
            )}
          </div>
        )}
      </div>
    </div>
  );
}

function ConnectionLabel({ st }: { st: ReturnType<typeof useProviderStatus> }) {
  switch (st.connection) {
    case "loading":
      return <span className="pv-conn pv-conn-muted">Checking…</span>;
    case "connected":
      return (
        <span className="pv-conn pv-conn-ok">
          <span className="dot dot-sm" aria-hidden />
          {st.viewer ? `Connected as ${st.viewer}` : "Connected"}
        </span>
      );
    case "error":
      return (
        <span className="pv-conn pv-conn-danger">
          <span className="dot dot-sm" aria-hidden />
          {st.status?.hasKey ? "Connection problem" : "Unavailable"}
        </span>
      );
    default:
      return <span className="pv-conn pv-conn-muted">Not connected</span>;
  }
}
