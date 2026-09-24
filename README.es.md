# Nodal

[![CI](https://github.com/Mazp17/nodal-agentos/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/Mazp17/nodal-agentos/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/Mazp17/nodal-agentos?include_prereleases&sort=semver&label=release)](https://github.com/Mazp17/nodal-agentos/releases)
[![Platform: macOS](https://img.shields.io/badge/platform-macOS-lightgrey?logo=apple)](#instalación)
[![License: MIT](https://img.shields.io/github/license/Mazp17/nodal-agentos)](LICENSE.md)

**Agent OS for Claude.** Planifica tareas en un board, delégalas a agentes de Claude Code y sigue cada run en todos tus repos.

[English](README.md)

![Board de Nodal](docs/screenshots/board.png)

> Alpha temprana, solo macOS. Proyecto independiente, sin relación con Anthropic.

## Instalación

1. Descarga el último `.dmg` desde [Releases](https://github.com/Mazp17/nodal-agentos/releases) (Apple Silicon e Intel) y arrastra Nodal a Aplicaciones.
2. Nodal no está notarizado por Apple, así que macOS bloquea la primera apertura. Corre una vez:

   ```bash
   xattr -dr com.apple.quarantine /Applications/Nodal.app
   ```

Necesitas [Claude Code](https://docs.claude.com/en/docs/claude-code) instalado y con sesión iniciada, y `git`. `gh` es opcional, para tareas que abren pull requests. Abre `claude` una vez en cada repo y acepta el diálogo de confianza antes de correr tareas ahí.

## Qué hace

- **Board de tareas** agrupadas en proyectos, cada uno con uno o más repos locales. Planes en markdown, criterios de aceptación, prioridad y etiquetas.
- **Delega** cada tarea a uno de tus agentes (`~/.claude/agents`, los del repo o de un plugin), a un workflow o a Claude a secas.
- **Aislado por defecto**: cada tarea tiene su propio worktree y rama de git, así varias corren a la vez en el mismo repo.
- **Termina** con los cambios sin commitear, con un commit o con un pull request.
- **Revisión automática** contra los criterios de aceptación: si pasa va a In Review, si no a Blocked con los hallazgos.
- **Una cola** con límite de concurrencia global, y cada run en vivo: fases, subagentes, transcript, tokens y diff.
- **Linear, opcional**: importa issues, las rutea a repos y mantiene los estados sincronizados. Hay más task managers planeados.

## Cómo funciona

Nodal maneja el CLI `claude` que ya tienes (`claude --bg`, `--agent`, `/<workflow>`) y lee el progreso de `claude agents --json` y de los archivos de sesión en `~/.claude/projects`. Esos archivos son un formato interno sin documentar, así que una actualización de Claude Code puede romper la lectura.

Todo queda en tu Mac: una base SQLite, los worktrees en `~/.nodal` y las API keys en el Llavero. Sin servidor ni telemetría.

## Compilar desde el código

Necesitas Node.js 20+, pnpm 10 y Rust estable.

```bash
git clone https://github.com/Mazp17/nodal-agentos.git nodal
cd nodal
pnpm install
pnpm tauri dev
```

Hecho con [Tauri 2](https://tauri.app) (Rust + SQLite) y React 19 + TypeScript. En [CONTRIBUTING.md](CONTRIBUTING.md) está cómo trabajar en el proyecto y en [RELEASING.md](RELEASING.md) cómo se publican las versiones.

## Licencia

[MIT](LICENSE.md) © Nodal contributors
