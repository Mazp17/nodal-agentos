//! Application/use-case layer: orchestrates the domain, the store and the ports
//! (implemented by nodal-host, nodal-linear and the shell) behind one `App` facade per
//! context. No tauri dependency.
#![forbid(unsafe_code)]

mod core;
mod flows;
#[cfg(test)]
mod testutil;

pub mod agent_api;
pub mod board;
pub mod chats;
pub mod execution;
pub mod sessions;
pub mod sources;
pub mod system;

pub use core::{AppError, Deps, Env, HubDeps};

use std::sync::Arc;

/// One facade per bounded context, built once at startup and shared as `Arc<App>`.
pub struct App {
    pub board: board::Board,
    pub execution: Arc<execution::Execution>,
    pub sessions: sessions::Sessions,
    pub sources: Arc<sources::SourcesHub>,
    pub chats: chats::Chats,
    pub agent_api: agent_api::AgentApi,
    pub system: system::System,
}

impl App {
    pub fn new(deps: Deps) -> Arc<App> {
        let core = Arc::new(core::Core {
            db: deps.db,
            env: deps.env,
            rt: deps.rt,
            clock: deps.clock,
            claude: deps.claude,
            sessions: deps.sessions,
            notifier: deps.notifier,
            chats: deps.chats,
        });
        let execution = execution::Execution::new(core.clone());
        Arc::new(App {
            board: board::Board::new(core.clone()),
            execution: execution.clone(),
            sessions: sessions::Sessions::new(core.clone()),
            sources: deps.sources,
            chats: chats::Chats::new(core.clone()),
            agent_api: agent_api::AgentApi::new(core.clone(), execution),
            system: system::System::new(core),
        })
    }

    /// Today's `work::init` queue loop: a pass (`Execution::pump`), `eprintln!("work: {e}")`
    /// on error, then sleep 5s, forever.
    pub async fn run_pump(self: Arc<Self>) {
        loop {
            if let Err(e) = self.execution.pump().await {
                eprintln!("work: {e}");
            }
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        }
    }

    /// Fires one queue pass in the background (today's `work::kick`).
    pub fn kick(self: &Arc<Self>) {
        self.execution.kick();
    }
}
