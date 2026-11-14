//! Tauri application library entry point.
//!
//! This module serves as the main entry point for the Tauri application.
//! Commands are declared once in `rpc` (both transports); the desktop-only
//! ones are in `desktop`. Shared types are in the `types` module.

#[cfg(test)]
mod approvals_suite;
mod audit;
mod bindings;
#[cfg(test)]
mod browser_launch_check;
#[cfg(test)]
mod browser_sites_suite;
mod cli;
mod cli_mascot;
#[cfg(test)]
mod connector_hub_suite;
mod db;
mod desktop;
mod engine;
mod headless;
mod http;
mod llm;
mod mcp_client;
mod model;
#[cfg(test)]
mod offline_suite;
mod rpc;
mod runtime;
mod sandbox;
#[cfg(test)]
mod sandbox_suite;
#[cfg(test)]
mod secrets_suite;
mod services;
mod types;
mod utils;
mod verify;

use tauri::{Manager, RunEvent, WindowEvent};

/// Application entry point. Sets up all plugins and initializes the app.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    cli::attach_parent_console();
    let options = match cli::CliOptions::from_env() {
        Ok(options) => options,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    if options.help {
        println!("{}", cli::USAGE);
        return;
    }
    if let Some(command) = &options.mascot {
        std::process::exit(cli_mascot::run(command));
    }
    if let Some(agent_id) = &options.verify {
        std::process::exit(verify::run(agent_id));
    }
    if let Some(command) = &options.publisher {
        std::process::exit(services::marketplace::signature::run_cli(command));
    }
    if let Some(force) = options.rebuild {
        std::process::exit(services::repo_service::rebuild_cli(force));
    }
    if options.headless {
        if let Err(e) = headless::run(options) {
            eprintln!("Nandi failed to start: {e}");
            std::process::exit(1);
        }
        return;
    }

    let builder = bindings::generate_bindings();
    let invoke_handler = builder.invoke_handler();

    // Export TypeScript bindings in debug builds
    #[cfg(debug_assertions)]
    bindings::export_ts_bindings();

    // Build with common plugins
    let mut app_builder = tauri::Builder::default();

    // Single instance plugin must be registered FIRST
    // When user tries to open a second instance, focus the existing window instead.
    // Skipped with NANDI_HOME, so instances with separate homes can run side by side
    #[cfg(desktop)]
    {
        if !utils::nandi_config::has_custom_home() {
            app_builder =
                app_builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
                    if let Some(window) = app.get_webview_window("main") {
                        let _ = window.set_focus();
                        let _ = window.unminimize();
                    }
                }));
        }
    }

    // Window state plugin - saves/restores window position and size
    #[cfg(desktop)]
    {
        app_builder = app_builder.plugin(
            tauri_plugin_window_state::Builder::new()
                .with_state_flags(tauri_plugin_window_state::StateFlags::all())
                // The companion keeps to a screen corner of its own
                .with_denylist(&[desktop::companion::COMPANION_LABEL])
                .build(),
        );
    }

    // Updater plugin for in-app updates
    #[cfg(desktop)]
    {
        app_builder = app_builder.plugin(tauri_plugin_updater::Builder::new().build());
    }

    app_builder = app_builder
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_notification::init())
        .plugin({
            #[allow(unused_mut)]
            let mut targets = vec![
                // Always log to stdout for development
                tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::Stdout),
                // A file in the app's log folder on every platform
                // (Windows: %LOCALAPPDATA%/com.nandi.app/logs/nandi.log)
                tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::LogDir {
                    file_name: Some("nandi".into()),
                }),
            ];
            // Log to webview console — excluded on Linux where the WebKitGTK webview
            // doesn't exist during setup(), causing app.emit() to deadlock on the IPC socket.
            #[cfg(not(target_os = "linux"))]
            targets.push(tauri_plugin_log::Target::new(
                tauri_plugin_log::TargetKind::Webview,
            ));
            // Nandi's own logs: Debug in development, Info in release.
            // Dependencies (HTTP stacks, keyring, Docker, AWS: aws_config
            // logs the access key ID at Info) only at Warn.
            let ours = if cfg!(debug_assertions) {
                log::LevelFilter::Debug
            } else {
                log::LevelFilter::Info
            };
            tauri_plugin_log::Builder::new()
                .level(log::LevelFilter::Warn)
                .level_for("nandi_lib", ours)
                .level_for("nandi", ours)
                .level_for("webview", ours)
                .max_file_size(5 * 1024 * 1024)
                .rotation_strategy(tauri_plugin_log::RotationStrategy::KeepSome(3))
                .targets(targets)
                .build()
        });

    app_builder
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_persisted_scope::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_os::init())
        .setup(move |app| {
            builder.mount_events(app);

            log::info!("Application starting up");
            log::debug!(
                "App handle initialized for package: {}",
                app.package_info().name
            );

            // Set up global shortcut plugin (without any shortcuts - we register them separately)
            #[cfg(desktop)]
            {
                use tauri_plugin_global_shortcut::Builder;

                app.handle().plugin(Builder::new().build())?;
            }

            let app_data_dir = app.path().app_data_dir()?;

            // The floating companion: the app's one global hotkey and its
            // pinned state. Its window is declared in tauri.conf.json.
            desktop::companion::setup(app.handle());

            // Nandi core: SQLite state, one long-lived orchestrator per agent,
            // and the event bus (fanning out to this webview and to WebSocket
            // clients). Independent of what the UI shows.
            let state = services::AppState::start(services::Config {
                app_data_dir,
                runtime: tauri::async_runtime::handle().inner().clone(),
                sinks: vec![std::sync::Arc::new(desktop::events::TauriSink(
                    app.handle().clone(),
                ))],
                secrets: services::secrets::Secrets::keychain(services::APP_IDENTIFIER),
                built_in_relay: services::relay::BuiltInRelay::of_this_build(),
            })?;

            // Container-facing egress proxy (LLM, AWS, HTTP)
            services::egress::start_server(&state);

            // Web transport (REST + WebSocket + frontend), token-protected.
            // A failure here is logged; the desktop app works without it.
            let web = utils::nandi_config::nandi_dir()
                .and_then(|dir| http::auth::WebToken::load_or_create(&dir.join("web_token")))
                .and_then(|token| {
                    let addr = tauri::async_runtime::block_on(http::serve(
                        state.clone(),
                        options.bind(),
                        token.clone(),
                    ))?;
                    Ok((addr, token))
                });
            match web {
                Ok((addr, token)) if options.announce() => {
                    let url = cli::access_url(addr, token.as_str());
                    println!("Nandi web access: {url}");
                    cli::record_access_url(&url);
                    log::info!("Web access enabled on {addr} (link printed to stdout)");
                }
                Ok((addr, _)) => {
                    log::info!("Web access on http://{addr} (token in ~/.nandi/web_token)");
                }
                Err(e) => log::error!("Web server not started: {e}"),
            }
            app.manage(state);

            // NOTE: Application menu is built from JavaScript for i18n support
            // See src/lib/menu.ts for the menu implementation

            Ok(())
        })
        .invoke_handler(invoke_handler)
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| match &event {
            // Hide the main window instead of quitting, so agents keep running and the
            // companion stays: it opens the main window again and has Quit. On macOS the
            // Dock icon reopens it too.
            RunEvent::WindowEvent {
                label,
                event: WindowEvent::CloseRequested { api, .. },
                ..
            } if label == "main" => {
                #[cfg(desktop)]
                {
                    api.prevent_close();

                    // Save window state before hiding
                    use tauri_plugin_window_state::{AppHandleExt, StateFlags};
                    if let Err(e) = app_handle.save_window_state(StateFlags::all()) {
                        log::warn!("Failed to save window state: {e}");
                    }

                    // Hide the window, not the app. app_handle.hide() calls NSApplication.hide()
                    // which sets system-level hidden state — showing an NSPanel while hidden
                    // causes macOS to unhide the entire app, including the main window.
                    if let Some(window) = app_handle.get_webview_window("main") {
                        let _ = window.hide();
                        log::info!("Main window hidden");
                    }
                    // No Dock to reopen it from: the companion is the way back
                    #[cfg(not(target_os = "macos"))]
                    desktop::companion::show_after_main_closed(app_handle);
                }
            }

            // macOS: Dock icon clicked — reopen the main window if it was hidden
            #[cfg(target_os = "macos")]
            RunEvent::Reopen { .. } => {
                if let Some(window) = app_handle.get_webview_window("main") {
                    if !window.is_visible().unwrap_or(true) {
                        let _ = window.show();

                        // The window-state plugin only auto-restores on app startup, not after
                        // a hide/show cycle. Without this the window can appear at stale coords.
                        use tauri_plugin_window_state::{StateFlags, WindowExt};
                        let _ = window.restore_state(StateFlags::all());

                        let _ = window.set_focus();
                        log::info!("Main window reopened from dock");
                    }
                }
            }

            // Cleanup on actual exit (Cmd+Q, menu Quit, or window close on non-macOS).
            // RunEvent::Exit fires reliably before the process exits, unlike ExitRequested
            // which doesn't fire for Cmd+Q on macOS (tauri-apps/tauri#9198).
            RunEvent::Exit => {
                log::info!("Application exiting — performing cleanup");

                // Stop orchestrators; persisted runs resume on next launch
                if let Some(state) = app_handle.try_state::<services::AppState>() {
                    state.shutdown();
                }

                // Unregister global shortcuts
                #[cfg(desktop)]
                {
                    use tauri_plugin_global_shortcut::GlobalShortcutExt;
                    if let Err(e) = app_handle.global_shortcut().unregister_all() {
                        log::warn!("Failed to unregister global shortcuts: {e}");
                    }
                }

                log::info!("Cleanup complete");
            }

            _ => {}
        });
}
