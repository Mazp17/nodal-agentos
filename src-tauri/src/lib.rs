mod activity;
mod adapters;
mod chats;
pub mod commands;
mod db;
mod domain;
mod events;
mod linear;
pub mod mcp;
mod migrate;
mod paths;
mod providers;
mod runs;
mod secrets;
mod updates;
mod util;
mod work;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let secrets = secrets::keychain();
    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .menu(updates::menu)
        .on_menu_event(updates::on_menu_event)
        .on_page_load(|webview, payload| {
            // A full reload doesn't tell the frontend's terminals to close: without this, any
            // `claude attach` started before the reload would keep running with nobody reading
            // its output. This only fires on a full page load, not on Vite's HMR in dev (which
            // patches modules in place), so it doesn't help with that case.
            if let tauri::webview::PageLoadEvent::Started = payload.event() {
                use tauri::Manager;
                if let Some(sessions) = webview.try_state::<runs::pty::PtySessions>() {
                    sessions.close_owned_by(webview.label());
                }
            }
        })
        .manage(linear::LinearState::new(secrets.clone()))
        .manage(secrets)
        .manage(mcp::commands::McpState::default())
        .manage(runs::pty::PtySessions::default())
        .setup(|app| {
            use std::sync::Arc;

            use tauri::Manager;

            let handle = app.handle().clone();
            app.manage(adapters::events::Events::new(handle.clone()));
            if paths::DEV {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.set_title("Nodal Dev");
                }
            }

            // Shared between the legacy states and the hub/app: one `http::Client` and one
            // `Secrets` cache (already built above, before `.manage`).
            let http = app.state::<linear::LinearState>().http().clone();
            let events = app.state::<adapters::events::Events>().inner().clone();
            let secrets = app.state::<secrets::Secrets>().inner().clone();
            let clock: Arc<dyn nodal_domain::ports::Clock> = Arc::new(nodal_host::adapters::SystemClock);
            let rt = tauri::async_runtime::handle().inner().clone();

            // The hub works without a database: provider, key and live-session commands do
            // too. Registered before the database opens, and always.
            let data_dir = paths::data_dir(&handle)?;
            let providers: Vec<Arc<dyn nodal_domain::ports::ProviderFactory>> =
                vec![Arc::new(nodal_linear::LinearFactory::new(http))];
            let opened = db::open(&data_dir.join(db::DB_FILE));
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

            // The only place that starts the background workers: the queue pump
            // (`commands::execution::setup`) with the MCP socket (`commands::mcp::setup`) and
            // the chat runtime (`commands::chats::runtime`). Without a database the app still
            // opens (to show the error): there is no pump, chats don't run and those commands
            // fail.
            match opened {
                Ok(db) => {
                    app.manage(db.clone());
                    let claude_dir = runs::claude_fs::claude_config_dir();
                    let sessions: Arc<dyn nodal_domain::ports::SessionFiles> = Arc::new(nodal_host::adapters::HostSessionFiles);
                    let claude: Arc<dyn nodal_domain::ports::ClaudeCli> = Arc::new(nodal_host::adapters::HostClaudeCli::new(rt.clone()));
                    // Builds the one `ChatProcesses` instance, manages it as the legacy
                    // `ChatState` and starts its reaper (today's chats setup) — unconditionally,
                    // like today, regardless of whether the env below can be built.
                    let chat_runtime = commands::chats::runtime(&handle, &db, &events, sessions.clone(), claude_dir.clone(), &rt);

                    let env_built = (|| -> Result<nodal_app::Env, String> {
                        Ok(nodal_app::Env {
                            data_dir: data_dir.clone(),
                            worktrees_root: util::paths::nodal_home()?.join("worktrees"),
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
                                claude,
                                sessions,
                                notifier: Arc::new(events),
                                chats: chat_runtime,
                                sources: hub.clone(),
                            });
                            app.manage(app_arc.clone());
                            let _ = commands::execution::setup(&handle, &app_arc, &db);
                            commands::chats::setup(&handle, &app_arc);
                        }
                        Err(e) => eprintln!("work: {e}"),
                    }
                }
                Err(e) => eprintln!("nodal.db: {e}"),
            }

            // Registered even without a database, so provider, key and live-session commands
            // (`SourcesHub`, `SessionReader`) still work.
            app.manage(hub.clone());
            app.manage(nodal_app::sessions::SessionReader::new(
                Arc::new(nodal_host::adapters::HostClaudeCli::new(rt)),
                Arc::new(nodal_host::adapters::HostSessionFiles),
            ));

            if let Err(e) = commands::sources::setup(&handle, &hub) {
                eprintln!("providers: {e}");
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::host::claude_version,
            commands::sessions::list_runs,
            commands::sessions::get_run_detail,
            commands::sessions::get_agent_transcript,
            commands::sessions::get_launch_blocker,
            commands::sessions::get_run_transcript,
            commands::sessions::external_sessions,
            commands::host::attach_run,
            commands::host::open_terminal_at,
            commands::pty::pty_attach,
            commands::pty::pty_write,
            commands::pty::pty_resize,
            commands::pty::pty_close,
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
            commands::execution::cleanup_worktree,
            commands::execution::merge_worktree,
            commands::execution::worktree_status,
            commands::board::list_executors,
            commands::board::list_hidden_executors,
            commands::board::set_executor_hidden,
            commands::execution::list_task_runs,
            commands::execution::list_runs_light,
            commands::execution::get_run,
            commands::execution::latest_runs_by_task,
            commands::execution::list_queue,
            commands::execution::work_summary,
            commands::execution::launch_task,
            commands::execution::hand_off,
            commands::execution::review_now,
            commands::execution::confirm_run,
            commands::execution::cancel_run,
            commands::execution::reorder_queue,
            commands::execution::run_diff,
            commands::execution::open_in_editor,
            commands::execution::open_worktree,
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
            commands::system::import_legacy_data,
            updates::updates_enabled,
            updates::restart_app,
            commands::mcp::mcp_status,
            commands::mcp::mcp_restart,
            commands::mcp::mcp_stop,
            commands::chats::list_chats,
            commands::chats::get_claude_defaults,
            commands::chats::create_chat,
            commands::chats::update_chat,
            commands::chats::delete_chat,
            commands::chats::send_chat_message,
            commands::chats::respond_chat_permission,
            commands::chats::interrupt_chat,
            commands::chats::stop_chat,
            commands::chats::get_chat_live,
            commands::chats::get_chat_transcript,
        ])
        .build(tauri::generate_context!())
        .expect("error while running tauri application")
        .run(|app_handle, event| {
            // Nothing else reaps embedded terminals on exit: without this, `claude attach`
            // child processes would outlive the app.
            if let tauri::RunEvent::Exit = event {
                use tauri::Manager;
                if let Some(sessions) = app_handle.try_state::<runs::pty::PtySessions>() {
                    sessions.close_all();
                }
            }
        });
}
