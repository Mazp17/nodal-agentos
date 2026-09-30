import React from "react";
import ReactDOM from "react-dom/client";
// JetBrains Mono bundled locally: the CSP (`default-src 'self'`) blocks Google Fonts.
import "@fontsource/jetbrains-mono/latin-400.css";
import "@fontsource/jetbrains-mono/latin-500.css";
import "./styles/tokens.css";
import "./styles/base.css";
import "./styles/tailwind.css";
import App from "./App";
import { migrateLegacyStorage } from "./shell/storage";
import { installZoomShortcuts } from "./shell/zoom";

// Before the first render: old preferences (`agent-desk.*`) move to `nodal.*`.
migrateLegacyStorage();
installZoomShortcuts();
// Tauri's file-drop handler is off (`dragDropEnabled: false`, so HTML5 drag works in the
// board): without this, a file dropped from Finder would navigate the webview to it.
for (const type of ["dragover", "drop"] as const) {
  window.addEventListener(type, (e) => {
    if (e.dataTransfer?.types.includes("Files")) e.preventDefault();
  });
}

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
