//! "Get your computer ready" with one go-ahead (phase 2, Windows and macOS).
//!
//! [`plan`] turns a [`MachineReport`] into the steps to run, in order:
//! WSL first (`wsl --update`, or `wsl --install` and a restart), then
//! Docker Desktop, then Start Docker. The user approves the whole list
//! once; [`run`] runs it step by step and checks the computer again after
//! each one. A step that fails stops the plan with what Nandi saw and the
//! official page to do it by hand.
//!
//! Every step is a fixed program with fixed arguments ([`command`]), never
//! a shell. Nandi itself is never elevated: `winget`, `wsl` and the
//! installers raise their own UAC or password prompt. Linux stays phase 1
//! (copyable commands), so its plan is empty.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use specta::Type;

use super::assess::links;
use super::{CheckId, CheckReason, CheckState, MachineCheck, MachineOs, MachineReport};

/// An install can take a while (Docker Desktop is about 500 MB)
pub const INSTALL_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// Lines of output kept for the card
const LOG_LINES: usize = 200;

/// One step of the plan
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum ReadyStep {
    /// `wsl --update` (Windows asks for admin rights)
    WslUpdate,
    /// `wsl --install --no-distribution` (Windows asks for admin rights)
    WslInstall,
    /// winget on Windows; Homebrew or Docker's `.dmg` on macOS
    InstallDocker,
    /// Launches Docker Desktop (no admin rights)
    StartDocker,
    /// After installing WSL: the user restarts the computer when ready
    Restart,
}

fn find(report: &MachineReport, id: CheckId) -> Option<&MachineCheck> {
    report.checks.iter().find(|c| c.id == id)
}

/// The steps that get this computer ready, in order. Empty when there's
/// nothing Nandi can run (ready, Linux, or only manual fixes left).
pub fn plan(report: &MachineReport) -> Vec<ReadyStep> {
    if !matches!(report.os, MachineOs::Windows | MachineOs::Macos) {
        return Vec::new();
    }
    let docker_missing = find(report, CheckId::DockerInstalled)
        .is_some_and(|c| c.state == CheckState::Missing && c.reason == CheckReason::NotInstalled);
    let mut steps = Vec::new();
    let mut restart = false;
    if report.os == MachineOs::Windows {
        match find(report, CheckId::Wsl).map(|c| c.state) {
            Some(CheckState::Missing) => {
                steps.push(ReadyStep::WslInstall);
                restart = true;
            }
            Some(CheckState::NeedsUpdate) => steps.push(ReadyStep::WslUpdate),
            // Docker needs a current WSL: update it first (owner, 2026-11-18)
            Some(CheckState::Ok) if docker_missing => steps.push(ReadyStep::WslUpdate),
            _ => {}
        }
    }
    if docker_missing {
        steps.push(ReadyStep::InstallDocker);
    }
    let start = find(report, CheckId::DockerRunning).is_some_and(|c| match c.reason {
        CheckReason::NotRunning => true,
        CheckReason::NeedsDocker => docker_missing,
        _ => false,
    });
    if restart {
        // Docker starts after the restart
        steps.push(ReadyStep::Restart);
    } else if start && !steps.is_empty() {
        steps.push(ReadyStep::StartDocker);
    }
    steps
}

/// The user approved `steps`: still what this computer needs?
pub fn approved(steps: &[ReadyStep], report: &MachineReport) -> Result<(), String> {
    let now = plan(report);
    if steps.is_empty() || steps != now.as_slice() {
        return Err(
            "This computer changed since Nandi asked, so nothing ran. Check again".to_string(),
        );
    }
    Ok(())
}

/// What's on this computer to install with
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tools {
    /// Windows: `winget` is there
    pub winget: bool,
    /// macOS: where `brew` is
    pub brew: Option<String>,
}

/// How a step runs: always a program with fixed arguments, never a shell
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepCommand {
    Run {
        program: String,
        args: Vec<String>,
    },
    /// Download Docker's `.dmg` into Downloads, then `open` it
    DownloadAndOpen {
        url: &'static str,
        to: PathBuf,
    },
    /// No installer Nandi can run (no winget): the official download page
    DownloadPage {
        url: &'static str,
    },
    StartDocker,
    /// Nothing to run: the user restarts the computer
    Restart,
}

fn program(program: &str, args: &[&str]) -> StepCommand {
    StepCommand::Run {
        program: program.to_string(),
        args: args.iter().map(|a| a.to_string()).collect(),
    }
}

pub const WINGET_DOCKER: &[&str] = &[
    "install",
    "-e",
    "--id",
    "Docker.DockerDesktop",
    "--accept-package-agreements",
    "--accept-source-agreements",
];

/// The command for `step` on this computer
pub fn command(
    step: ReadyStep,
    report: &MachineReport,
    tools: &Tools,
    downloads: &Path,
) -> StepCommand {
    match step {
        ReadyStep::WslUpdate => program("wsl", &["--update"]),
        ReadyStep::WslInstall => program("wsl", &["--install", "--no-distribution"]),
        ReadyStep::InstallDocker => match report.os {
            MachineOs::Windows if tools.winget => program("winget", WINGET_DOCKER),
            MachineOs::Windows => StepCommand::DownloadPage {
                url: links::WINDOWS_INSTALL,
            },
            _ => match &tools.brew {
                Some(brew) => program(brew, &["install", "--cask", "docker"]),
                None => StepCommand::DownloadAndOpen {
                    url: if report.apple_silicon {
                        links::MAC_ARM_DMG
                    } else {
                        links::MAC_INTEL_DMG
                    },
                    to: downloads.join("Docker.dmg"),
                },
            },
        },
        ReadyStep::StartDocker => StepCommand::StartDocker,
        ReadyStep::Restart => StepCommand::Restart,
    }
}

/// The official page to do `step` by hand
pub fn manual_link(step: ReadyStep, os: MachineOs) -> &'static str {
    match (step, os) {
        (ReadyStep::InstallDocker, MachineOs::Windows) => links::WINDOWS_INSTALL,
        (ReadyStep::InstallDocker, _) => links::MAC_INSTALL,
        (ReadyStep::StartDocker, _) => links::TROUBLESHOOT,
        _ => links::WSL_INSTALL,
    }
}

/// After a step, the fresh check decides: did it work? Steps whose effect
/// the check can see are judged by it, not by the exit code (winget exits
/// non-zero when Docker is already there).
fn verified(step: ReadyStep, report: &MachineReport) -> Option<Result<(), String>> {
    let id = match step {
        ReadyStep::WslUpdate => CheckId::Wsl,
        ReadyStep::InstallDocker => CheckId::DockerInstalled,
        _ => return None,
    };
    let check = find(report, id)?;
    Some(if check.state == CheckState::Ok {
        Ok(())
    } else {
        Err(format!("Nandi checked again and saw: {}", check.detail))
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum ReadyStepStatus {
    Waiting,
    Running,
    Done,
    /// Nandi did its part; the user finishes it (the `.dmg`, the download
    /// page, the restart)
    HandedOver,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ReadyStepProgress {
    pub step: ReadyStep,
    pub status: ReadyStepStatus,
}

/// What the user finishes, when the plan hands over
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum ReadyHandover {
    /// macOS: Docker's `.dmg` is open: drag Docker to Applications
    Dmg,
    /// Windows without winget: download Docker Desktop from docker.com
    DownloadPage,
    /// WSL is installed: restart the computer
    Restart,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ReadyFailure {
    pub step: ReadyStep,
    /// What Nandi saw
    pub detail: String,
    /// The official page to do it by hand
    pub link: String,
}

/// How "Get your computer ready" is going (`machine://ready`)
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ReadyProgress {
    pub running: bool,
    pub steps: Vec<ReadyStepProgress>,
    /// The commands' output, last lines
    pub log: Vec<String>,
    pub handover: Option<ReadyHandover>,
    /// A link that goes with the handover (the download page)
    pub handover_link: Option<String>,
    pub failure: Option<ReadyFailure>,
    /// The check after the last step
    pub report: Option<MachineReport>,
}

impl ReadyProgress {
    pub fn starting(steps: &[ReadyStep]) -> Self {
        Self {
            running: true,
            steps: steps
                .iter()
                .map(|&step| ReadyStepProgress {
                    step,
                    status: ReadyStepStatus::Waiting,
                })
                .collect(),
            ..Self::default()
        }
    }

    fn line(&mut self, line: &str) {
        self.log.push(line.to_string());
        if self.log.len() > LOG_LINES {
            self.log.remove(0);
        }
    }
}

/// What a step's output reports to
pub type OnLine<'a> = &'a (dyn Fn(&str) + Send + Sync);

/// Runs the commands. The real one is in `install`; tests use a fake that
/// records the commands, so nothing is installed by `cargo test`.
pub trait Runner: Send + Sync {
    fn tools(&self) -> impl std::future::Future<Output = Tools> + Send;
    fn downloads(&self) -> PathBuf;
    fn run(
        &self,
        program: &str,
        args: &[String],
        on_line: OnLine<'_>,
    ) -> impl std::future::Future<Output = Result<(), String>> + Send;
    fn download(
        &self,
        url: &str,
        to: &Path,
        on_line: OnLine<'_>,
    ) -> impl std::future::Future<Output = Result<(), String>> + Send;
    fn start_docker(&self) -> impl std::future::Future<Output = Result<(), String>> + Send;
    /// A fresh check of this computer
    fn check(&self) -> impl std::future::Future<Output = MachineReport> + Send;
}

/// Runs the approved `steps` in order on the computer `report` describes,
/// telling `update` after each change. Stops at the first failure or
/// handover.
pub async fn run<R: Runner>(
    runner: &R,
    steps: &[ReadyStep],
    report: MachineReport,
    update: &(dyn Fn(&ReadyProgress) + Send + Sync),
) -> ReadyProgress {
    let progress = Mutex::new(ReadyProgress::starting(steps));
    let change = |f: &dyn Fn(&mut ReadyProgress)| {
        let mut progress = progress.lock().unwrap_or_else(|e| e.into_inner());
        f(&mut progress);
        update(&progress);
    };
    let on_line = |line: &str| change(&|p| p.line(line));
    let tools = runner.tools().await;
    let downloads = runner.downloads();
    let mut report = report;
    for (index, &step) in steps.iter().enumerate() {
        change(&|p| p.steps[index].status = ReadyStepStatus::Running);
        let outcome: Result<Option<(ReadyHandover, Option<&str>)>, String> =
            match command(step, &report, &tools, &downloads) {
                StepCommand::Run { program, args } => {
                    runner.run(&program, &args, &on_line).await.map(|()| None)
                }
                StepCommand::DownloadAndOpen { url, to } => {
                    match runner.download(url, &to, &on_line).await {
                        Ok(()) => runner
                            .run("open", &[to.to_string_lossy().into_owned()], &on_line)
                            .await
                            .map(|()| Some((ReadyHandover::Dmg, None))),
                        Err(e) => Err(e),
                    }
                }
                StepCommand::DownloadPage { url } => {
                    Ok(Some((ReadyHandover::DownloadPage, Some(url))))
                }
                StepCommand::StartDocker => runner.start_docker().await.map(|()| None),
                StepCommand::Restart => Ok(Some((ReadyHandover::Restart, None))),
            };
        report = runner.check().await;
        let outcome = match (verified(step, &report), outcome) {
            (_, Ok(Some(handover))) => Ok(Some(handover)),
            (Some(Ok(())), _) => Ok(None),
            (Some(Err(seen)), Err(error)) => Err(format!("{error}. {seen}")),
            (Some(Err(seen)), Ok(None)) => Err(seen),
            (None, outcome) => outcome,
        };
        let checked = report.clone();
        change(&|p| p.report = Some(checked.clone()));
        match outcome {
            Ok(None) => change(&|p| p.steps[index].status = ReadyStepStatus::Done),
            Ok(Some((handover, link))) => {
                change(&|p| {
                    p.steps[index].status = ReadyStepStatus::HandedOver;
                    p.handover = Some(handover);
                    p.handover_link = link.map(str::to_string);
                });
                break;
            }
            Err(detail) => {
                change(&|p| {
                    p.steps[index].status = ReadyStepStatus::Failed;
                    p.failure = Some(ReadyFailure {
                        step,
                        detail: detail.clone(),
                        link: manual_link(step, report.os).to_string(),
                    });
                });
                break;
            }
        }
    }
    change(&|p| p.running = false);
    progress.into_inner().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::machine::MachineAction;

    fn check(id: CheckId, state: CheckState, reason: CheckReason) -> MachineCheck {
        MachineCheck {
            id,
            state,
            reason,
            detail: format!("{id:?} {state:?}"),
            actions: Vec::<MachineAction>::new(),
        }
    }

    fn ok(id: CheckId) -> MachineCheck {
        check(id, CheckState::Ok, CheckReason::Ready)
    }

    /// A Windows or macOS report with every check ok
    fn ready(os: MachineOs) -> MachineReport {
        let mut checks = vec![ok(CheckId::DockerInstalled), ok(CheckId::DockerRunning)];
        if os == MachineOs::Windows {
            checks.push(ok(CheckId::Wsl));
        }
        checks.push(ok(CheckId::Disk));
        MachineReport {
            os,
            arch: "x86_64".into(),
            apple_silicon: false,
            distro: None,
            checks,
            docker_ready: true,
            ready: true,
            checked_at: String::new(),
            plan: Vec::new(),
        }
    }

    fn set(report: &mut MachineReport, new: MachineCheck) {
        let at = report.checks.iter().position(|c| c.id == new.id).unwrap();
        report.checks[at] = new;
    }

    fn docker_missing(report: &mut MachineReport) {
        set(
            report,
            check(
                CheckId::DockerInstalled,
                CheckState::Missing,
                CheckReason::NotInstalled,
            ),
        );
        set(
            report,
            check(
                CheckId::DockerRunning,
                CheckState::Missing,
                CheckReason::NeedsDocker,
            ),
        );
    }

    fn docker_stopped(report: &mut MachineReport) {
        set(
            report,
            check(
                CheckId::DockerRunning,
                CheckState::Stopped,
                CheckReason::NotRunning,
            ),
        );
    }

    fn wsl(report: &mut MachineReport, state: CheckState, reason: CheckReason) {
        set(report, check(CheckId::Wsl, state, reason));
    }

    use ReadyStep::*;

    #[test]
    fn plan_for_every_windows_combination() {
        let windows = || ready(MachineOs::Windows);
        assert!(plan(&windows()).is_empty());

        let mut r = windows();
        docker_missing(&mut r);
        assert_eq!(
            plan(&r),
            [WslUpdate, InstallDocker, StartDocker],
            "WSL current: updated first"
        );

        wsl(&mut r, CheckState::NeedsUpdate, CheckReason::OldWsl);
        assert_eq!(plan(&r), [WslUpdate, InstallDocker, StartDocker]);
        wsl(&mut r, CheckState::NeedsUpdate, CheckReason::KernelUpdate);
        assert_eq!(plan(&r), [WslUpdate, InstallDocker, StartDocker]);

        wsl(&mut r, CheckState::Missing, CheckReason::NotInstalled);
        assert_eq!(
            plan(&r),
            [WslInstall, InstallDocker, Restart],
            "Docker starts after the restart"
        );

        wsl(&mut r, CheckState::Unknown, CheckReason::NotChecked);
        assert_eq!(
            plan(&r),
            [InstallDocker, StartDocker],
            "an unreadable WSL isn't touched"
        );

        // Docker installed
        let mut r = windows();
        wsl(&mut r, CheckState::NeedsUpdate, CheckReason::OldWsl);
        assert_eq!(plan(&r), [WslUpdate]);
        docker_stopped(&mut r);
        assert_eq!(plan(&r), [WslUpdate, StartDocker]);
        wsl(&mut r, CheckState::Missing, CheckReason::NotInstalled);
        assert_eq!(plan(&r), [WslInstall, Restart]);

        // Only stopped: the Start Docker button, not a plan
        let mut r = windows();
        docker_stopped(&mut r);
        assert!(plan(&r).is_empty());
        // Unable to start, or starting: manual fixes and waiting
        set(
            &mut r,
            check(
                CheckId::DockerRunning,
                CheckState::Blocked,
                CheckReason::DesktopFailed,
            ),
        );
        assert!(plan(&r).is_empty());
        set(
            &mut r,
            check(
                CheckId::DockerRunning,
                CheckState::Stopped,
                CheckReason::Starting,
            ),
        );
        assert!(plan(&r).is_empty());
    }

    #[test]
    fn plan_for_every_macos_and_linux_combination() {
        let mut r = ready(MachineOs::Macos);
        assert!(plan(&r).is_empty());
        docker_missing(&mut r);
        assert_eq!(plan(&r), [InstallDocker, StartDocker], "no WSL on a Mac");
        // The Intel build on Apple silicon is a manual fix
        let mut r = ready(MachineOs::Macos);
        set(
            &mut r,
            check(
                CheckId::DockerInstalled,
                CheckState::NeedsUpdate,
                CheckReason::WrongBuild,
            ),
        );
        docker_stopped(&mut r);
        assert!(plan(&r).is_empty());

        // Linux stays phase 1
        let mut r = ready(MachineOs::Linux);
        docker_missing(&mut r);
        assert!(plan(&r).is_empty());
    }

    #[test]
    fn approval_is_for_the_plan_the_check_finds() {
        let mut r = ready(MachineOs::Windows);
        docker_missing(&mut r);
        assert!(approved(&[WslUpdate, InstallDocker, StartDocker], &r).is_ok());
        assert!(
            approved(&[InstallDocker], &r).is_err(),
            "only what the check says"
        );
        assert!(approved(&[], &ready(MachineOs::Windows)).is_err());
    }

    #[test]
    fn commands_are_fixed_programs_and_arguments() {
        let downloads = Path::new("/Users/ada/Downloads");
        let windows = ready(MachineOs::Windows);
        let winget = Tools {
            winget: true,
            brew: None,
        };
        let args = |command: StepCommand| match command {
            StepCommand::Run { program, args } => (program, args),
            other => panic!("{other:?}"),
        };
        assert_eq!(
            args(command(WslUpdate, &windows, &winget, downloads)),
            ("wsl".to_string(), vec!["--update".to_string()])
        );
        assert_eq!(
            args(command(WslInstall, &windows, &winget, downloads)),
            (
                "wsl".to_string(),
                vec!["--install".into(), "--no-distribution".into()]
            )
        );
        assert_eq!(
            args(command(InstallDocker, &windows, &winget, downloads)),
            (
                "winget".to_string(),
                [
                    "install",
                    "-e",
                    "--id",
                    "Docker.DockerDesktop",
                    "--accept-package-agreements",
                    "--accept-source-agreements"
                ]
                .map(String::from)
                .to_vec()
            )
        );
        // No winget: the official page, nothing else
        assert_eq!(
            command(InstallDocker, &windows, &Tools::default(), downloads),
            StepCommand::DownloadPage {
                url: links::WINDOWS_INSTALL
            }
        );

        let mut mac = ready(MachineOs::Macos);
        let brew = Tools {
            winget: false,
            brew: Some("/opt/homebrew/bin/brew".into()),
        };
        assert_eq!(
            args(command(InstallDocker, &mac, &brew, downloads)),
            (
                "/opt/homebrew/bin/brew".to_string(),
                ["install", "--cask", "docker"].map(String::from).to_vec()
            )
        );
        assert_eq!(
            command(InstallDocker, &mac, &Tools::default(), downloads),
            StepCommand::DownloadAndOpen {
                url: links::MAC_INTEL_DMG,
                to: downloads.join("Docker.dmg")
            }
        );
        mac.apple_silicon = true;
        assert_eq!(
            command(InstallDocker, &mac, &Tools::default(), downloads),
            StepCommand::DownloadAndOpen {
                url: links::MAC_ARM_DMG,
                to: downloads.join("Docker.dmg")
            }
        );
        assert_eq!(
            command(StartDocker, &mac, &brew, downloads),
            StepCommand::StartDocker
        );
        // No argument is ever a shell
        for step in [WslUpdate, WslInstall, InstallDocker] {
            if let StepCommand::Run { program, args } = command(step, &windows, &winget, downloads)
            {
                assert!(!["cmd", "powershell", "sh", "bash"].contains(&program.as_str()));
                assert!(args.iter().all(|a| !a.contains(['&', '|', ';', '>'])));
            }
        }
    }

    /// Records the commands; the checks it answers come from a script
    struct Fake {
        tools: Tools,
        ran: Mutex<Vec<String>>,
        fail: Option<&'static str>,
        checks: Mutex<Vec<MachineReport>>,
    }

    impl Fake {
        fn new(tools: Tools, checks: Vec<MachineReport>) -> Self {
            Self {
                tools,
                ran: Mutex::default(),
                fail: None,
                checks: Mutex::new(checks),
            }
        }

        fn ran(&self) -> Vec<String> {
            self.ran.lock().unwrap().clone()
        }
    }

    impl Runner for Fake {
        async fn tools(&self) -> Tools {
            self.tools.clone()
        }

        fn downloads(&self) -> PathBuf {
            PathBuf::from("/Users/ada/Downloads")
        }

        async fn run(
            &self,
            program: &str,
            args: &[String],
            on_line: OnLine<'_>,
        ) -> Result<(), String> {
            let line = format!("{program} {}", args.join(" "));
            on_line(&line);
            self.ran.lock().unwrap().push(line.clone());
            match self.fail {
                Some(fail) if line.contains(fail) => Err(format!("{program} exited with 1")),
                _ => Ok(()),
            }
        }

        async fn download(&self, url: &str, to: &Path, _: OnLine<'_>) -> Result<(), String> {
            self.ran
                .lock()
                .unwrap()
                .push(format!("download {url} {}", to.display()));
            Ok(())
        }

        async fn start_docker(&self) -> Result<(), String> {
            self.ran.lock().unwrap().push("start docker".into());
            Ok(())
        }

        async fn check(&self) -> MachineReport {
            let mut checks = self.checks.lock().unwrap();
            if checks.len() > 1 {
                checks.remove(0)
            } else {
                checks[0].clone()
            }
        }
    }

    fn statuses(progress: &ReadyProgress) -> Vec<ReadyStepStatus> {
        progress.steps.iter().map(|s| s.status).collect()
    }

    #[tokio::test]
    async fn runs_the_windows_plan_in_order_checking_after_each_step() {
        let mut before = ready(MachineOs::Windows);
        docker_missing(&mut before);
        let mut installed = ready(MachineOs::Windows);
        docker_stopped(&mut installed);
        let fake = Fake::new(
            Tools {
                winget: true,
                brew: None,
            },
            vec![before.clone(), installed, ready(MachineOs::Windows)],
        );
        let updates = Mutex::new(0);
        let steps = plan(&before);
        let done = run(&fake, &steps, before, &|_| *updates.lock().unwrap() += 1).await;
        assert_eq!(
            fake.ran(),
            [
                "wsl --update",
                "winget install -e --id Docker.DockerDesktop --accept-package-agreements --accept-source-agreements",
                "start docker"
            ]
        );
        assert_eq!(statuses(&done), [ReadyStepStatus::Done; 3]);
        assert!(!done.running && done.failure.is_none());
        assert!(done.report.unwrap().docker_ready);
        assert!(done.log.contains(&"wsl --update".to_string()));
        assert!(*updates.lock().unwrap() > 6);
    }

    #[tokio::test]
    async fn a_failed_step_stops_the_plan_with_what_nandi_saw() {
        let mut before = ready(MachineOs::Windows);
        docker_missing(&mut before);
        let mut fake = Fake::new(
            Tools {
                winget: true,
                brew: None,
            },
            vec![before.clone()],
        );
        fake.fail = Some("winget");
        let done = run(&fake, &plan(&before), before, &|_| {}).await;
        assert_eq!(
            fake.ran().len(),
            2,
            "Start Docker never ran: {:?}",
            fake.ran()
        );
        assert_eq!(
            statuses(&done),
            [
                ReadyStepStatus::Done,
                ReadyStepStatus::Failed,
                ReadyStepStatus::Waiting
            ]
        );
        let failure = done.failure.unwrap();
        assert_eq!(failure.step, InstallDocker);
        assert_eq!(failure.link, links::WINDOWS_INSTALL);
        assert!(
            failure.detail.contains("winget exited with 1"),
            "{}",
            failure.detail
        );
        assert!(
            failure.detail.contains("DockerInstalled Missing"),
            "{}",
            failure.detail
        );
    }

    #[tokio::test]
    async fn the_check_decides_when_the_exit_code_disagrees() {
        // winget exits non-zero when Docker is there already
        let mut before = ready(MachineOs::Windows);
        docker_missing(&mut before);
        let mut fake = Fake::new(
            Tools {
                winget: true,
                brew: None,
            },
            vec![ready(MachineOs::Windows)],
        );
        fake.fail = Some("winget");
        let done = run(&fake, &plan(&before), before.clone(), &|_| {}).await;
        assert_eq!(statuses(&done), [ReadyStepStatus::Done; 3]);

        // wsl --update says it worked, and WSL is still old
        let mut old = ready(MachineOs::Windows);
        wsl(&mut old, CheckState::NeedsUpdate, CheckReason::OldWsl);
        let fake = Fake::new(Tools::default(), vec![old.clone()]);
        let done = run(&fake, &plan(&old), old, &|_| {}).await;
        assert_eq!(statuses(&done), [ReadyStepStatus::Failed]);
        assert_eq!(done.failure.unwrap().link, links::WSL_INSTALL);
    }

    #[tokio::test]
    async fn hands_over_for_the_restart_the_dmg_and_no_winget() {
        let mut before = ready(MachineOs::Windows);
        docker_missing(&mut before);
        wsl(&mut before, CheckState::Missing, CheckReason::NotInstalled);
        let fake = Fake::new(
            Tools {
                winget: true,
                brew: None,
            },
            vec![ready(MachineOs::Windows)],
        );
        let done = run(&fake, &plan(&before), before, &|_| {}).await;
        assert_eq!(done.handover, Some(ReadyHandover::Restart));
        assert_eq!(
            statuses(&done),
            [
                ReadyStepStatus::Done,
                ReadyStepStatus::Done,
                ReadyStepStatus::HandedOver
            ]
        );

        let mut before = ready(MachineOs::Windows);
        set(
            &mut before,
            check(
                CheckId::DockerInstalled,
                CheckState::Missing,
                CheckReason::NotInstalled,
            ),
        );
        let mut missing = before.clone();
        wsl(&mut missing, CheckState::Ok, CheckReason::Ready);
        let fake = Fake::new(Tools::default(), vec![missing]);
        let done = run(&fake, &plan(&before), before, &|_| {}).await;
        assert_eq!(fake.ran(), ["wsl --update"], "no winget: nothing else runs");
        assert_eq!(done.handover, Some(ReadyHandover::DownloadPage));
        assert_eq!(done.handover_link.as_deref(), Some(links::WINDOWS_INSTALL));

        let mut mac = ready(MachineOs::Macos);
        mac.apple_silicon = true;
        docker_missing(&mut mac);
        let fake = Fake::new(Tools::default(), vec![mac.clone()]);
        let done = run(&fake, &plan(&mac), mac, &|_| {}).await;
        let dmg = fake.downloads().join("Docker.dmg");
        assert_eq!(
            fake.ran(),
            [
                format!("download {} {}", links::MAC_ARM_DMG, dmg.display()),
                format!("open {}", dmg.display())
            ]
        );
        assert_eq!(done.handover, Some(ReadyHandover::Dmg));
        assert_eq!(
            statuses(&done),
            [ReadyStepStatus::HandedOver, ReadyStepStatus::Waiting]
        );
    }
}
