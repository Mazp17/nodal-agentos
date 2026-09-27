// Dev-only tools (`import.meta.env.DEV`): not part of release builds' settings.

import { useToast, type ToastTone } from "../../ui/Toasts";
import { SectionHead } from "./SettingsView";
import "./debug.css";

const TONES: ToastTone[] = ["info", "ok", "warn", "danger", "accent", "muted"];

export function DebugSettings() {
  const toast = useToast();
  return (
    <>
      <SectionHead title="Debug" text="Only visible in development builds." />
      <div className="panel settings-card">
        <div className="settings-row settings-row-top">
          <div className="settings-row-text">
            <span className="settings-row-title">Toasts</span>
            <span className="settings-row-hint">One per tone. Errors stay until dismissed.</span>
          </div>
          <div className="debug-actions">
            {TONES.map((tone) => (
              <button key={tone} type="button" className="btn" onClick={() => toast(`Test toast · ${tone}`, "A sample body.", tone)}>
                {tone}
              </button>
            ))}
            <button
              type="button"
              className="btn"
              onClick={() =>
                toast(
                  "Multi-line toast",
                  "First line of a longer body that wraps across the card to check spacing.\nSecond line after a break.",
                  "danger",
                )
              }
            >
              multi-line
            </button>
          </div>
        </div>
      </div>
    </>
  );
}
