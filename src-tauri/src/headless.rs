//! `--headless`: the Nandi core and web server without any Tauri window.
//! Stops on Ctrl-C.

use std::sync::Arc;

use crate::cli::{access_url, record_access_url, CliOptions};
use crate::http::{self, auth::WebToken};
use crate::services::{secrets::Secrets, AppState, Config, APP_IDENTIFIER};
use crate::utils::nandi_config;

/// Minimal stderr logger (tauri-plugin-log isn't running without Tauri).
struct StderrLogger;

impl log::Log for StderrLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            eprintln!(
                "[{}] {}: {}",
                record.level(),
                record.target(),
                record.args()
            );
        }
    }

    fn flush(&self) {}
}

static LOGGER: StderrLogger = StderrLogger;

pub fn run(options: CliOptions) -> Result<(), String> {
    let _ = log::set_logger(&LOGGER);
    log::set_max_level(if cfg!(debug_assertions) {
        log::LevelFilter::Debug
    } else {
        log::LevelFilter::Info
    });

    let runtime = tokio::runtime::Runtime::new().map_err(|e| e.to_string())?;
    runtime.block_on(async {
        // Same folder Tauri's app_data_dir() resolves to, so preferences
        // are shared with the desktop app
        let app_data_dir = dirs::data_dir()
            .ok_or("Could not resolve the data directory")?
            .join(APP_IDENTIFIER);
        let state = AppState::start(Config {
            app_data_dir,
            runtime: tokio::runtime::Handle::current(),
            sinks: Vec::<Arc<dyn crate::services::events::EventSink>>::new(),
            secrets: Secrets::keychain(APP_IDENTIFIER),
            built_in_relay: crate::services::relay::BuiltInRelay::of_this_build(),
        })?;
        crate::services::egress::start_server(&state);

        let token = WebToken::load_or_create(&nandi_config::nandi_dir()?.join("web_token"))?;
        let addr = http::serve(state.clone(), options.bind(), token.clone()).await?;
        let url = access_url(addr, token.as_str());
        let mascot = crate::services::mascot::read(&nandi_config::nandi_dir()?);
        let agents = state.store().read(|repo| repo.agents().count());
        println!(
            "{} Running headless. Open:\n\n    {url}\n",
            crate::services::mascot::line_up(&mascot.name, agents)
        );
        record_access_url(&url);
        if addr.ip().is_unspecified() {
            println!("Listening on all interfaces (--server): anyone on your network with the link can connect.");
        }

        tokio::signal::ctrl_c().await.map_err(|e| e.to_string())?;
        log::info!("Shutting down");
        state.shutdown();
        Ok(())
    })
}
