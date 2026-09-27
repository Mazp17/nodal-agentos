// Chat notices: an answer finished or a tool needs approval while the chat isn't on screen.
// A system notification when Nodal is in the background; a toast when it's in front, where
// macOS files the notification away without a banner. What's unread is tracked in `stream.ts`.

import { getCurrentWindow } from "@tauri-apps/api/window";
import { isPermissionGranted, requestPermission, sendNotification } from "@tauri-apps/plugin-notification";

/** Checked once per launch (on desktop the plugin always reports it granted). */
let allowed: Promise<boolean> | null = null;

const permitted = () =>
  (allowed ??= isPermissionGranted()
    .then((granted) => granted || requestPermission().then((p) => p === "granted"))
    .catch((e: unknown) => {
      allowed = null;
      console.error("notification permission", e);
      return false;
    }));

/** One line of an answer for the notification body, without Markdown marks. */
export function preview(text: string | null, max = 160): string {
  const one = (text ?? "").replace(/[*_`#>]+/g, "").replace(/\s+/g, " ").trim();
  return one.length > max ? `${one.slice(0, max - 1)}…` : one;
}

/**
 * Nodal's window is the one in front. Asked to Tauri rather than trusting `document.hasFocus()`
 * inside the WKWebView with the app in the background.
 */
export const appFocused = (): Promise<boolean> =>
  getCurrentWindow()
    .isFocused()
    .catch(() => document.hasFocus());

type InApp = (title: string, body: string) => void;
let inApp: InApp | null = null;

/** Where notices go while Nodal is in front (the app's toasts); `null` to stop. */
export function setInAppNotice(show: InApp | null) {
  inApp = show;
}

/** Posts a notice titled with the chat's name. */
export async function notifyChat(title: string, body: string, focused: boolean) {
  if (inApp && focused) {
    inApp(title, body);
    return;
  }
  // Failures surface as the plugin's own unhandled rejection: it doesn't return a promise.
  if (await permitted()) sendNotification({ title, body });
}
