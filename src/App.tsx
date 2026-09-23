import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  configApi,
  linearApi,
  toLinearError,
  type AppConfig,
  type Board,
  type LinearError,
  type Team,
} from "./features/linear/api";
import { BoardView, ErrorState } from "./features/linear/Board";
import { Onboarding, SettingsView } from "./features/linear/Settings";
import "./App.css";

const TEAM_FILTER_KEY = "agent-desk.teamFilter";
const DEFAULT_CONFIG: AppConfig = { repos: [], concurrency: 3 };

// localStorage puede no estar disponible; el filtro es sólo una comodidad.
function readTeamFilter(): string {
  try {
    return localStorage.getItem(TEAM_FILTER_KEY) ?? "";
  } catch {
    return "";
  }
}
function writeTeamFilter(v: string) {
  try {
    localStorage.setItem(TEAM_FILTER_KEY, v);
  } catch {
    /* ignorar */
  }
}

type View = "board" | "settings";

function App() {
  const [cli, setCli] = useState<{ ok: boolean; text: string } | null>(null);
  const [keyConfigured, setKeyConfigured] = useState<boolean | null>(null);
  const [keyError, setKeyError] = useState<LinearError | null>(null);
  const [view, setView] = useState<View>("board");
  const [teams, setTeams] = useState<Team[]>([]);
  const [teamFilter, setTeamFilter] = useState<string>(readTeamFilter);
  const [board, setBoard] = useState<Board | null>(null);
  const [boardError, setBoardError] = useState<LinearError | null>(null);
  const [loading, setLoading] = useState(false);
  const [config, setConfig] = useState<AppConfig>(DEFAULT_CONFIG);
  // Hasta que get_config responda bien, Ajustes no deja guardar (pisaría el archivo).
  const [configLoaded, setConfigLoaded] = useState(false);
  const [configError, setConfigError] = useState<string | null>(null);

  useEffect(() => {
    invoke<string>("claude_version")
      .then((text) => setCli({ ok: true, text }))
      .catch((err) => setCli({ ok: false, text: String(err) }));
    configApi
      .get()
      .then((c) => {
        setConfig(c);
        setConfigLoaded(true);
      })
      .catch((e) => setConfigError(String(e)));
  }, []);

  const checkKey = useCallback(() => {
    setKeyError(null);
    linearApi
      .keyStatus()
      .then((s) => setKeyConfigured(s.configured))
      .catch((e) => setKeyError(toLinearError(e)));
  }, []);
  useEffect(checkKey, [checkKey]);

  // Descarta respuestas viejas si se cambia el filtro con un fetch en vuelo.
  const requestId = useRef(0);
  const loadBoard = useCallback(async () => {
    const id = ++requestId.current;
    setLoading(true);
    setBoardError(null);
    try {
      // allSettled: si falla el board (p. ej. filtro viejo) igual queremos la lista de
      // teams para poder cambiar el filtro.
      const [ts, b] = await Promise.allSettled([
        linearApi.teams(),
        linearApi.board(teamFilter ? [teamFilter] : null),
      ]);
      if (id !== requestId.current) return;
      if (ts.status === "fulfilled") setTeams(ts.value);
      if (b.status === "fulfilled") setBoard(b.value);
      const failed = b.status === "rejected" ? b.reason : ts.status === "rejected" ? ts.reason : null;
      if (failed !== null) {
        const err = toLinearError(failed);
        if (err.kind === "missingKey") setKeyConfigured(false);
        setBoardError(err);
      }
    } finally {
      if (id === requestId.current) setLoading(false);
    }
  }, [teamFilter]);

  useEffect(() => {
    if (keyConfigured) loadBoard();
  }, [keyConfigured, loadBoard]);

  // Si el team guardado ya no existe (key de otro workspace), volver a "todos".
  useEffect(() => {
    if (teamFilter && teams.length > 0 && !teams.some((t) => t.id === teamFilter)) {
      setTeamFilter("");
      writeTeamFilter("");
    }
  }, [teams, teamFilter]);

  function onKeyCleared() {
    requestId.current++;
    setLoading(false);
    setBoard(null);
    setTeams([]);
    setBoardError(null);
    setKeyConfigured(false);
  }

  let content;
  if (keyError) {
    content = <ErrorState error={keyError} onRetry={checkKey} onSettings={checkKey} />;
  } else if (keyConfigured === null) {
    content = <div className="state-panel muted">Cargando…</div>;
  } else if (!keyConfigured) {
    content = <Onboarding onSaved={() => setKeyConfigured(true)} />;
  } else if (view === "settings") {
    content = (
      <SettingsView
        teams={teams}
        issues={board?.issues ?? []}
        config={config}
        configLoaded={configLoaded}
        keyConfigured={keyConfigured}
        onConfigSaved={(c) => {
          setConfig(c);
          setConfigLoaded(true);
          setConfigError(null);
        }}
        onKeySaved={loadBoard}
        onKeyCleared={onKeyCleared}
      />
    );
  } else if (boardError && !board) {
    content = (
      <ErrorState error={boardError} onRetry={loadBoard} onSettings={() => setView("settings")} />
    );
  } else if (!board) {
    content = <div className="state-panel muted">Cargando issues…</div>;
  } else if (board.issues.length === 0) {
    content = (
      <div className="state-panel">
        <h2>No hay issues</h2>
        <p>Nada abierto ni cerrado en los últimos 14 días{teamFilter ? " en este team" : ""}.</p>
      </div>
    );
  } else {
    content = <BoardView issues={board.issues} config={config} />;
  }

  const showBoardControls = keyConfigured && view === "board";

  return (
    <div className="app">
      <header className="topbar">
        <div className="topbar-left">
          <h1>Agent Desk</h1>
          {keyConfigured && (
            <nav className="tabs">
              <button className={view === "board" ? "active" : ""} onClick={() => setView("board")}>
                Board
              </button>
              <button
                className={view === "settings" ? "active" : ""}
                onClick={() => setView("settings")}
              >
                Ajustes
              </button>
            </nav>
          )}
          {showBoardControls && (
            <>
              <select
                className="team-filter"
                value={teamFilter}
                onChange={(e) => {
                  setTeamFilter(e.target.value);
                  writeTeamFilter(e.target.value);
                }}
                aria-label="Filtrar por team"
              >
                <option value="">Todos los teams</option>
                {teams.map((t) => (
                  <option key={t.id} value={t.id}>
                    {t.key} · {t.name}
                  </option>
                ))}
              </select>
              <button className="btn" onClick={loadBoard} disabled={loading}>
                {loading ? "Actualizando…" : "Actualizar"}
              </button>
            </>
          )}
        </div>
        <span className={`cli ${cli?.ok === false ? "cli-error" : ""}`}>
          {cli === null ? "buscando claude…" : cli.ok ? `claude ${cli.text}` : cli.text}
        </span>
      </header>
      {showBoardControls && board && boardError && (
        <div className="banner error">
          No se pudo actualizar: {boardError.message}
        </div>
      )}
      {showBoardControls && board?.truncated && (
        <div className="banner">Hay más issues de las que se muestran (tope de 2000). Filtrá por team.</div>
      )}
      {configError && <div className="banner error">{configError}</div>}
      {content}
    </div>
  );
}

export default App;
