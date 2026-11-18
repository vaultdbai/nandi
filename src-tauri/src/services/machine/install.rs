//! Runs the approved "Get your computer ready" plan for real, in the
//! background: one plan at a time, progress on `machine://ready`, the
//! go-ahead in the decision record. What runs and in which order is
//! [`ready`](super::ready) (pure, tested with a fake runner).

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use tokio::io::AsyncReadExt;

use super::assess::Cmd;
use super::parse::LineSplitter;
use super::ready::{self, OnLine, ReadyProgress, ReadyStep, Runner, Tools, INSTALL_TIMEOUT};
use super::{probe, MachineOs, MachineReport};
use crate::services::events::{self, NandiEvent};
use crate::services::AppState;

impl NandiEvent for ReadyProgress {
    const NAME: &'static str = "machine://ready";
}

static PROGRESS: LazyLock<Mutex<ReadyProgress>> = LazyLock::new(Mutex::default);

fn progress() -> std::sync::MutexGuard<'static, ReadyProgress> {
    PROGRESS.lock().unwrap_or_else(|e| e.into_inner())
}

/// How the last (or running) plan is going
pub fn status() -> ReadyProgress {
    progress().clone()
}

/// Go ahead: checks the computer again, makes sure `steps` is still what
/// it needs, records the go-ahead and runs the steps in the background.
pub async fn start(state: &AppState, steps: Vec<ReadyStep>) -> Result<(), String> {
    if progress().running {
        return Err("Nandi is already getting this computer ready".to_string());
    }
    let report = super::check(true).await;
    ready::approved(&steps, &report)?;
    {
        let mut progress = progress();
        if progress.running {
            return Err("Nandi is already getting this computer ready".to_string());
        }
        *progress = ReadyProgress::starting(&steps);
    }
    crate::services::inbox::audit::record(
        state,
        crate::services::inbox::audit::Decided::new(
            "computerSetup",
            uuid::Uuid::new_v4().to_string(),
            "approved",
        )
        .subject(format!("Get your computer ready: {}", describe(&steps))),
    );
    let bus = state.bus().clone();
    tauri::async_runtime::spawn(async move {
        let runner = Real { os: report.os };
        let update = |p: &ReadyProgress| {
            *progress() = p.clone();
            events::emit(&bus, p);
        };
        let done = ready::run(&runner, &steps, report, &update).await;
        match &done.failure {
            Some(failure) => log::warn!(
                "Getting the computer ready stopped at {:?}: {}",
                failure.step,
                failure.detail
            ),
            None => log::info!("Got the computer ready: {}", describe(&steps)),
        }
    });
    Ok(())
}

/// The steps in a few words, for the decision record
fn describe(steps: &[ReadyStep]) -> String {
    steps
        .iter()
        .map(|step| match step {
            ReadyStep::WslUpdate => "wsl --update",
            ReadyStep::WslInstall => "wsl --install --no-distribution",
            ReadyStep::InstallDocker => "install Docker Desktop",
            ReadyStep::StartDocker => "start Docker",
            ReadyStep::Restart => "restart the computer",
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// The real commands: fixed arguments, no shell, never elevated (the
/// installers ask the OS for admin rights themselves)
struct Real {
    os: MachineOs,
}

impl Runner for Real {
    async fn tools(&self) -> Tools {
        match self.os {
            MachineOs::Windows => Tools {
                winget: !matches!(
                    probe::run("winget", &["--version"], Duration::from_secs(15)).await,
                    Cmd::NotFound
                ),
                brew: None,
            },
            MachineOs::Macos => {
                // An app opened from the Dock doesn't get Homebrew's PATH
                let known = ["/opt/homebrew/bin/brew", "/usr/local/bin/brew"]
                    .into_iter()
                    .find(|path| Path::new(path).is_file())
                    .map(str::to_string);
                Tools {
                    winget: false,
                    brew: known,
                }
            }
            _ => Tools::default(),
        }
    }

    fn downloads(&self) -> PathBuf {
        dirs::download_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_else(std::env::temp_dir)
    }

    async fn run(&self, program: &str, args: &[String], on_line: OnLine<'_>) -> Result<(), String> {
        on_line(&format!("> {program} {}", args.join(" ")));
        stream(program, args, on_line).await
    }

    async fn download(&self, url: &str, to: &Path, on_line: OnLine<'_>) -> Result<(), String> {
        download(url, to, on_line).await
    }

    async fn start_docker(&self) -> Result<(), String> {
        probe::start_docker(self.os).await
    }

    async fn check(&self) -> MachineReport {
        super::check(true).await
    }
}

/// Runs `program args`, passing each line of its output on as it comes
async fn stream(program: &str, args: &[String], on_line: OnLine<'_>) -> Result<(), String> {
    let mut command = tokio::process::Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    let mut child = command.spawn().map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => format!("{program} isn't on this computer"),
        _ => format!("{program} didn't start: {e}"),
    })?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let last = Mutex::new(None::<String>);
    let finished = tokio::time::timeout(INSTALL_TIMEOUT, async {
        tokio::join!(pump(stdout, on_line, &last), pump(stderr, on_line, &last));
        child.wait().await
    })
    .await;
    let last = last.into_inner().unwrap_or_else(|e| e.into_inner());
    match finished {
        Err(_) => Err(format!(
            "{program} didn't finish in {} minutes",
            INSTALL_TIMEOUT.as_secs() / 60
        )),
        Ok(Err(e)) => Err(format!("{program} failed: {e}")),
        Ok(Ok(status)) if status.success() => Ok(()),
        Ok(Ok(status)) => {
            let code = status.code().unwrap_or(-1);
            Err(match last {
                Some(line) => format!("{program} exited with {code}: {line}"),
                None => format!("{program} exited with {code}"),
            })
        }
    }
}

/// Passes on each line `pipe` prints; remembers the last one (for the
/// error when the command fails)
async fn pump<R: tokio::io::AsyncRead + Unpin>(
    pipe: Option<R>,
    on_line: OnLine<'_>,
    last: &Mutex<Option<String>>,
) {
    let Some(mut pipe) = pipe else {
        return;
    };
    let mut lines = LineSplitter::default();
    let mut buffer = [0u8; 4096];
    let pass = |found: Vec<String>| {
        for line in found {
            on_line(&line);
            *last.lock().unwrap_or_else(|e| e.into_inner()) = Some(line);
        }
    };
    while let Ok(read) = pipe.read(&mut buffer).await {
        if read == 0 {
            break;
        }
        pass(lines.push(&buffer[..read]));
    }
    pass(lines.finish());
}

/// Downloads `url` to `to`, reporting every 10%
async fn download(url: &str, to: &Path, on_line: OnLine<'_>) -> Result<(), String> {
    use tokio::io::AsyncWriteExt;
    on_line(&format!("Downloading {url}"));
    let client = reqwest::Client::builder()
        .timeout(INSTALL_TIMEOUT)
        .build()
        .map_err(|e| e.to_string())?;
    let mut response = client
        .get(url)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|e| format!("The download didn't start: {e}"))?;
    let total = response.content_length();
    let partial = to.with_extension("dmg.part");
    let mut file = tokio::fs::File::create(&partial)
        .await
        .map_err(|e| format!("Couldn't write {}: {e}", partial.display()))?;
    let mut got: u64 = 0;
    let mut shown = 0;
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| format!("The download stopped: {e}"))?
    {
        file.write_all(&chunk)
            .await
            .map_err(|e| format!("Couldn't write {}: {e}", partial.display()))?;
        got += chunk.len() as u64;
        if let Some(total) = total.filter(|t| *t > 0) {
            let tenth = got * 10 / total;
            if tenth > shown {
                shown = tenth;
                on_line(&format!("{}%", tenth * 10));
            }
        }
    }
    file.flush().await.map_err(|e| e.to_string())?;
    drop(file);
    tokio::fs::rename(&partial, to)
        .await
        .map_err(|e| format!("Couldn't save {}: {e}", to.display()))?;
    on_line(&format!("Saved {}", to.display()));
    Ok(())
}
