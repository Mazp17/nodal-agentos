# Nodal

**Agent OS for Claude.** Una app de escritorio para planificar trabajo, delegarlo a agentes de Claude Code y ver cómo corren en todos tus proyectos.

[English](README.md)

> Estado: alpha temprana, solo macOS. Puede tener asperezas y cambios que rompan compatibilidad.
>
> Nodal es un proyecto open source independiente. No está afiliado ni respaldado por Anthropic.

## Para qué sirve

Claude Code funciona muy bien en una terminal. Se vuelve difícil de seguir cuando tienes varios repos, varias tareas y varios agentes a la vez: qué está corriendo y dónde, qué run espera un permiso, cuál terminó y necesita revisión, a qué issue de Linear corresponde.

Nodal es la sala de control para eso. Tienes tus tareas en un board, asignas cada una a un ejecutor y Nodal lanza Claude Code en segundo plano, aísla el trabajo, lo revisa y mantiene sincronizado el estado de la tarea.

## Qué hace

- **Proyectos locales con varios repos.** Un proyecto agrupa uno o más repos git locales. Cada tarea pertenece exactamente a un repo.
- **Tareas con plan.** Escribes el plan en markdown o apuntas a un `.md` del repo. Sumas criterios de aceptación, prioridad y labels.
- **Delegación a un ejecutor.** Asignas la tarea a:
  - un **agente** que ya tengas (`~/.claude/agents`, `.claude/agents` del repo o un plugin), por ejemplo `frontend-developer`;
  - un **workflow** (`~/.claude/workflows` o los del repo), por ejemplo un pipeline de plan → implementación → verificación → revisión;
  - **Claude** a secas.
- **Aislamiento por tarea.** Por defecto cada tarea tiene su propio worktree de git y su rama (`~/.nodal/worktrees/…`, `nodal/<tarea>`), así que varias tareas pueden correr a la vez sobre el mismo repo. También puedes hacer que una tarea trabaje directo en la carpeta del repo.
- **Formas de cerrar.** Dejar los cambios sin commitear, commitearlos, o commitear, pushear y abrir un pull request.
- **Revisión automática.** Cuando un agente termina, un revisor en solo lectura verifica el trabajo contra los criterios de aceptación. Si pasa, la tarea va a In Review; si no, a Blocked con los hallazgos, y decides tú cómo sigue.
- **Traspasos.** Pasas una tarea de un ejecutor a otro sobre la misma rama (por ejemplo `frontend-developer` → `code-reviewer`). La tarea guarda toda la cadena de pasos.
- **Una sola cola global.** Un límite de concurrencia para todos los runs, con una cola visible que puedes reordenar.
- **Seguimiento de cada run.** Fases, subagentes, transcripts, tokens, el diff del worktree y las sesiones que Claude tiene abiertas en cada repo, incluso las que no lanzó Nodal.
- **Task managers, opcionales.** Importas issues de Linear y quedan vinculadas: Nodal empuja los cambios de estado (In Progress, In Review, Blocked) y deja un comentario de cierre. El mapeo de estados de Linear a los de Nodal lo propone Nodal y lo confirmas tú. Puedes asignar issues a repos por proyecto de Linear o por label. Asana, Azure DevOps y GitHub Issues están planeados.

Nodal funciona sin ningún task manager.

## Cómo funciona

Nodal no reemplaza a Claude Code: maneja el CLI `claude` que ya tienes instalado.

- Los runs se lanzan con `claude --bg`, con `--agent <nombre>` o como `/<workflow> <args>` según el ejecutor.
- El progreso se lee de `claude agents --json` y de los archivos de sesión que Claude Code escribe en `~/.claude/projects`. **Esos archivos son un formato interno y sin documentar**, aislado en un único módulo de parseo; una actualización de Claude Code puede romperlo.
- Todo es local: una base SQLite en la carpeta de datos de la app, los planes al lado y los worktrees en `~/.nodal`. Las API keys de los task managers se guardan en el Llavero de macOS. Nodal no tiene servidor ni envía telemetría.

## Requisitos

- macOS (la integración con el Llavero y con Terminal por ahora es solo de macOS).
- [Claude Code](https://docs.claude.com/en/docs/claude-code) instalado y con sesión iniciada (`claude` en el `PATH` o en `~/.local/bin`).
- `git`. `gh` (GitHub CLI) si quieres que las tareas abran pull requests.
- Claude Code tiene que confiar en un repo para que Nodal pueda correr tareas ahí: abre `claude` una vez en el repo y acepta el diálogo de confianza. La pantalla Diagnostics de Nodal muestra qué repos son de confianza.

Para compilar desde el código fuente también necesitas Node.js 20+, pnpm 10 y Rust estable.

## Instalación

Descarga el último `Nodal_<versión>_universal.dmg` (Apple Silicon e Intel) desde [Releases](https://github.com/Mazp17/nodal-agentos/releases), ábrelo y arrastra Nodal a Aplicaciones.

Nodal no está notarizado por Apple, así que macOS bloquea la primera apertura ("Nodal está dañado" o "no se puede abrir"). Quita la marca de cuarentena una vez:

```bash
xattr -dr com.apple.quarantine /Applications/Nodal.app
```

## Cómo empezar

```bash
git clone https://github.com/Mazp17/nodal-agentos.git nodal
cd nodal
pnpm install
pnpm tauri dev
```

La primera vez, Nodal te pide crear un proyecto, agregar un repo y, si quieres, conectar Linear con una API key personal.

Para generar la app empaquetada:

```bash
pnpm tauri build
```

## Stack

- [Tauri 2](https://tauri.app) con backend en Rust (`src-tauri/`): SQLite con `rusqlite`, la cola de runs, los worktrees y la sincronización con task managers.
- Frontend en React 19 + TypeScript + Vite (`src/`).

En [CONTRIBUTING.md](CONTRIBUTING.md) está la estructura del proyecto y cómo trabajar en él.

## Licencia

[MIT](LICENSE.md) © Nodal contributors
