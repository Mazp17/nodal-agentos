import React from "react";
import ReactDOM from "react-dom/client";
// JetBrains Mono bundled locally: the CSP (`default-src 'self'`) blocks Google Fonts.
import "@fontsource/jetbrains-mono/latin-400.css";
import "@fontsource/jetbrains-mono/latin-500.css";
import "./styles/tokens.css";
import "./styles/base.css";
import App from "./App";
import { migrateLegacyStorage } from "./shell/storage";

// Before the first render: old preferences (`agent-desk.*`) move to `nodal.*`.
migrateLegacyStorage();

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
