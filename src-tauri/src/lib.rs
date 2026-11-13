mod adapters;
pub mod commands;
mod menu;
mod paths;
mod secrets;
pub(crate) mod telemetry;
mod updates;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    telemetry::init();
    let secrets = secrets::keychain();
    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .menu(menu::menu)
        .on_menu_event(menu::on_menu_event)
        .manage(commands::linear::LinearState::new(secrets.clone()))
        .manage(secrets)
        .manage(commands::mcp::McpState::default())
        .setup(|app| {
            use std::sync::Arc;

            use tauri::Manager;

            let handle = app.handle().clone();
            app.manage(adapters::events::Events::new(handle.clone()));

            // Ola 0 instrumentation: one `info` log line a minute with the latency/spawn/IPC
            // counters (negligible against the S0 budget of < 2 wakeups/s), so a profiling
            // session doesn't need to wait for app exit to see a snapshot.
            tauri::async_runtime::spawn(async {
                let mut tick = tokio::time::interval(std::time::Duration::from_secs(60));
                tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                loop {
                    tick.tick().await;
                    telemetry::log_metrics_snapshot();
                }
            });

            if paths::DEV {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.set_title("Nodal Dev");
                }
            }

            // Shared between the legacy states and the hub/app: one `http::Client` and one
            // `Secrets` cache (already built above, before `.manage`).
            let http = app.state::<commands::linear::LinearState>().http().clone();
            let events = app.state::<adapters::events::Events>().inner().clone();
            let secrets = app.state::<secrets::Secrets>().inner().clone();
            let clock: Arc<dyn nodal_domain::ports::Clock> = Arc::new(nodal_host::adapters::SystemClock);
            let rt = tauri::async_runtime::handle().inner().clone();

            // The hub works without a database: provider, key and live-session commands do
            // too. Registered before the database opens, and always.
            let data_dir = paths::data_dir(&handle)?;
            let providers: Vec<Arc<dyn nodal_domain::ports::ProviderFactory>> =
                vec![Arc::new(nodal_linear::LinearFactory::new(http))];
            let opened = nodal_store::Db::open(&data_dir.join(nodal_store::DB_FILE));
            let hub = nodal_app::sources::SourcesHub::new(nodal_app::HubDeps {
                db: opened.as_ref().ok().cloned(),
                data_dir: data_dir.clone(),
                rt: rt.clone(),
                clock: clock.clone(),
                secrets,
                providers: nodal_app::sources::registry::ProviderRegistry::new(providers),
                notifier: Arc::new(events.clone()),
                plans: Arc::new(nodal_host::adapters::HostPlanFiles),
            });

            // The only place that starts the background workers: the MCP socket
            // (`commands::mcp::setup`) and the chat runtime (`commands::chats::runtime`).
            // Without a database the app still opens (to show the error): chats don't run and
            // those commands fail.
            match opened {
                Ok(db) => {
                    app.manage(db.clone());
                    let claude_dir = nodal_host::claude::fs::paths::claude_config_dir();
                    let sessions: Arc<dyn nodal_domain::ports::SessionFiles> = Arc::new(nodal_host::adapters::HostSessionFiles);
                    // Builds the one `ChatProcesses` instance and starts its reaper (today's
                    // chats setup) — unconditionally, like today, regardless of whether the env
                    // below can be built.
                    let chat_runtime = commands::chats::runtime(&handle, &db, &events, sessions.clone(), claude_dir.clone(), &rt);

                    let env_built = (|| -> Result<nodal_app::Env, String> {
                        Ok(nodal_app::Env {
                            data_dir: data_dir.clone(),
                            worktrees_root: nodal_host::paths::nodal_home()?.join("worktrees"),
                            claude_dir,
                            plans: Arc::new(nodal_host::adapters::HostPlanFiles),
                            fs: Arc::new(nodal_host::adapters::HostLocalFs),
                            git: Arc::new(nodal_host::adapters::HostGit),
                            claude_config: Arc::new(nodal_host::adapters::HostClaudeConfig),
                        })
                    })();
                    match env_built {
                        Ok(env) => {
                            let app_arc = nodal_app::App::new(nodal_app::Deps {
                                db: db.clone(),
                                env,
                                rt: rt.clone(),
                                clock: clock.clone(),
                                sessions,
                                notifier: Arc::new(events),
                                chats: chat_runtime,
                                sources: hub.clone(),
                            });
                            app.manage(app_arc.clone());
                            commands::mcp::setup(&handle, &app_arc);
                            commands::chats::setup(&handle, &app_arc);
                        }
                        Err(e) => eprintln!("work: {e}"),
                    }
                }
                Err(e) => eprintln!("nodal.db: {e}"),
            }

            // Registered even without a database, so provider and key commands still work.
            app.manage(hub.clone());

            if let Err(e) = commands::sources::setup(&handle, &hub) {
                eprintln!("providers: {e}");
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::host::claude_version,
            commands::host::open_terminal_at,
            commands::host::claude_usage,
            commands::host::resolve_git_root,
            commands::host::scan_git_repos,
            commands::host::git_version,
            commands::host::repo_trust,
            commands::linear::linear_issue_detail,
            commands::board::list_projects,
            commands::board::create_project,
            commands::board::update_project,
            commands::board::delete_project,
            commands::board::list_repos,
            commands::board::add_repo,
            commands::board::update_repo,
            commands::board::delete_repo,
            commands::board::list_tasks,
            commands::board::get_task,
            commands::board::create_task,
            commands::board::update_task,
            commands::board::delete_task,
            commands::board::move_task,
            commands::board::reorder_tasks,
            commands::board::read_task_plan,
            commands::board::list_task_relations,
            commands::board::add_task_relation,
            commands::board::remove_task_relation,
            commands::worktrees::cleanup_worktree,
            commands::worktrees::remove_repo_worktree,
            commands::worktrees::merge_worktree,
            commands::worktrees::worktree_status,
            commands::board::list_hidden_executors,
            commands::board::set_executor_hidden,
            commands::board::get_settings,
            commands::board::set_settings,
            commands::sources::provider_status,
            commands::sources::provider_set_key,
            commands::sources::provider_clear_key,
            commands::sources::provider_scopes,
            commands::sources::list_source_links,
            commands::sources::create_source_link,
            commands::sources::update_source_link,
            commands::sources::delete_source_link,
            commands::sources::unlink_task,
            commands::sources::source_states,
            commands::sources::save_state_map,
            commands::sources::provider_list_importable,
            commands::sources::import_tasks,
            commands::sources::sync_now,
            commands::sources::source_rule_projects,
            commands::sources::preview_rule_import,
            commands::sources::import_rule,
            commands::sources::resolve_moved_task,
            commands::system::repo_branch,
            commands::system::list_repo_worktrees,
            commands::system::revert_repo_file,
            commands::system::list_repo_dir,
            commands::system::read_repo_file,
            commands::system::write_repo_file,
            commands::system::repo_changes,
            commands::system::list_repo_files,
            commands::system::repo_commits,
            commands::system::repo_commit_diff,
            commands::system::repo_commit,
            commands::system::repo_open_pr,
            commands::system::import_legacy_data,
            updates::updates_enabled,
            updates::restart_app,
            commands::mcp::mcp_status,
            commands::mcp::mcp_restart,
            commands::mcp::mcp_stop,
            commands::chats::list_chats,
            commands::chats::get_claude_defaults,
            commands::chats::default_chat_system_prompt,
            commands::chats::list_chat_agents,
            commands::chats::create_chat,
            commands::chats::update_chat,
            commands::chats::delete_chat,
            commands::chats::send_chat_message,
            commands::chats::respond_chat_permission,
            commands::chats::interrupt_chat,
            commands::chats::rewind_chat,
            commands::chats::stop_chat,
            commands::chats::get_chat_live,
            commands::chats::get_chat_transcript,
            commands::chats::list_chat_subagents,
            commands::chats::get_chat_subagent_transcript,
            commands::chats::chat_workspace,
            commands::chats::start_task_chat,
            commands::chats::attach_chat_channel,
            commands::chats::detach_chat_channel,
        ])
        .build(tauri::generate_context!())
        .expect("error while running tauri application")
        .run(|_, event| {
            if let tauri::RunEvent::Exit = event {
                telemetry::log_metrics_snapshot();
            }
        });
}
