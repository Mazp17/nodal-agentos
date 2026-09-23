import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import "./App.css";
import { RunsPanel } from "./features/runs/RunsPanel";

// Placeholder hasta conectar Linear: las columnas reales salen de los estados de cada team.
const COLUMNS = ["Todo", "In Progress", "In Review", "Blocked", "Done"];

function App() {
  const [cli, setCli] = useState<{ ok: boolean; text: string } | null>(null);

  useEffect(() => {
    invoke<string>("claude_version")
      .then((text) => setCli({ ok: true, text }))
      .catch((err) => setCli({ ok: false, text: String(err) }));
  }, []);

  return (
    <div className="app">
      <header className="topbar">
        <h1>Agent Desk</h1>
        <span className={`cli ${cli?.ok === false ? "cli-error" : ""}`}>
          {cli === null ? "buscando claude…" : cli.ok ? `claude ${cli.text}` : cli.text}
        </span>
      </header>
      <main className="board">
        {COLUMNS.map((col) => (
          <section key={col} className="column">
            <h2>
              {col} <span className="count">0</span>
            </h2>
            <p className="empty">Sin issues</p>
          </section>
        ))}
      </main>
      <RunsPanel />
    </div>
  );
}

export default App;
