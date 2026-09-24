import { useId, useState, type ReactNode } from "react";
import { providerSetKey, type ProviderStatus } from "../../domain/api";
import { errorText, setProviderStatus } from "../../domain/hooks/providers";
import { PROVIDERS } from "./meta";
import "./providers.css";

/** Cuadradito de marca del proveedor (neutro: sin logos de terceros). */
export function ProviderMark() {
  return <span className="pv-mark" aria-hidden />;
}

export function Switch({
  checked,
  onChange,
  label,
  disabled,
}: {
  checked: boolean;
  onChange: (v: boolean) => void;
  label: string;
  disabled?: boolean;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      disabled={disabled}
      className={`pv-switch${checked ? " on" : ""}`}
      onClick={() => onChange(!checked)}
    >
      <span className="pv-switch-knob" />
    </button>
  );
}

export interface SegOption<T extends string> {
  value: T;
  label: string;
}

export function Segmented<T extends string>({
  value,
  options,
  onChange,
  label,
  disabled,
}: {
  value: T | null;
  options: SegOption<T>[];
  onChange: (v: T) => void;
  label: string;
  disabled?: boolean;
}) {
  return (
    <div className="segmented pv-seg" role="radiogroup" aria-label={label}>
      {options.map((o) => (
        <button
          key={o.value}
          type="button"
          role="radio"
          aria-checked={value === o.value}
          disabled={disabled}
          className={`pv-seg-opt${value === o.value ? " on" : ""}`}
          onClick={() => onChange(o.value)}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}

/** Fila etiqueta + control, como en Project settings. */
export function Field({ label, children, top }: { label: string; children: ReactNode; top?: boolean }) {
  return (
    <div className={`pv-field${top ? " top" : ""}`}>
      <span className="pv-field-label">{label}</span>
      <div className="pv-field-body">{children}</div>
    </div>
  );
}

/**
 * Pegar una key y validarla contra el proveedor ("Save and test"). Solo se guarda si es
 * válida; la key nunca vuelve del backend.
 */
export function KeyForm({
  provider,
  onSaved,
  autoFocus,
}: {
  provider: string;
  onSaved?: (status: ProviderStatus) => void;
  autoFocus?: boolean;
}) {
  const info = PROVIDERS.find((p) => p.id === provider);
  const [key, setKey] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const id = useId();

  const save = async () => {
    if (!key.trim() || busy) return;
    setBusy(true);
    setError(null);
    try {
      const status = await providerSetKey(provider, key.trim());
      setProviderStatus(status);
      setKey("");
      onSaved?.(status);
    } catch (err) {
      setError(errorText(err));
    } finally {
      setBusy(false);
    }
  };

  return (
    <form
      className="pv-keyform"
      onSubmit={(e) => {
        e.preventDefault();
        void save();
      }}
    >
      <div className="pv-keyrow">
        <label htmlFor={id} className="sr-only">
          {info?.name ?? provider} API key
        </label>
        <input
          id={id}
          type="password"
          autoComplete="off"
          spellCheck={false}
          className={`input input-mono pv-keyinput${error ? " input-invalid" : ""}`}
          placeholder={info?.keyPlaceholder ?? "API key"}
          value={key}
          onChange={(e) => {
            setKey(e.target.value);
            setError(null);
          }}
          aria-invalid={!!error}
          aria-describedby={`${id}-msg`}
          data-autofocus={autoFocus || undefined}
          autoFocus={autoFocus}
        />
        <button type="submit" className="btn btn-primary" disabled={!key.trim() || busy}>
          {busy ? "Testing…" : "Save and test"}
        </button>
      </div>
      <span id={`${id}-msg`} className={`pv-hint${error ? " pv-hint-error" : ""}`} role={error ? "alert" : undefined}>
        {error ?? `Keys live in the macOS Keychain, never on disk. ${info?.keyHelp ?? ""}`.trim()}
      </span>
    </form>
  );
}

/** Confirmación en línea (sin modal) para acciones destructivas. */
export function InlineConfirm({
  text,
  confirmLabel,
  busy,
  onConfirm,
  onCancel,
}: {
  text: string;
  confirmLabel: string;
  busy?: boolean;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  return (
    <div className="pv-confirm" role="group" aria-label={confirmLabel}>
      <span className="pv-confirm-text">{text}</span>
      <button type="button" className="btn btn-ghost btn-sm" onClick={onCancel} disabled={busy}>
        Cancel
      </button>
      <button type="button" className="btn btn-danger btn-sm" onClick={onConfirm} disabled={busy} data-autofocus>
        {busy ? "Working…" : confirmLabel}
      </button>
    </div>
  );
}
