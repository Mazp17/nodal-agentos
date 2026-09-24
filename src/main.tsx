import React from "react";
import ReactDOM from "react-dom/client";
// JetBrains Mono empaquetada localmente: la CSP (`default-src 'self'`) bloquea Google Fonts.
import "@fontsource/jetbrains-mono/latin-400.css";
import "@fontsource/jetbrains-mono/latin-500.css";
import "./styles/tokens.css";
import "./styles/base.css";
import App from "./App";
import { migrateLegacyStorage } from "./shell/storage";

// Antes del primer render: las preferencias viejas (`agent-desk.*`) pasan a `nodal.*`.
migrateLegacyStorage();

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
