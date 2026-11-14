//! One description of a sandboxed container, for every container Nandi
//! starts: agents, harness CLIs, connectors, the headless browser, tool
//! installs, the egress relay and the LiteLLM gateway.
//!
//! A [`SandboxSpec`] starts hardened and with resource limits, and the
//! callers only say what differs. It renders to `docker run` arguments
//! ([`SandboxSpec::to_cli_args`]) or to a bollard create body
//! ([`SandboxSpec::to_bollard`]), so the two can't drift apart.
//!
//! Always on, with no way to turn them off: every capability dropped
//! (`--cap-drop=ALL`), `no-new-privileges`, a memory, CPU and process
//! limit. On unless a caller says otherwise: a read-only root filesystem,
//! the images' own non-root user ([`AgentUser`]), no network.

use std::collections::{BTreeMap, HashMap};

use bollard::models::{ContainerCreateBody, HostConfig, Mount, MountType, PortBinding};

use crate::runtime::AgentUser;

/// Container limits when skill.md doesn't set `memory_mb` / `cpu`, and for
/// every container that doesn't set its own
pub const DEFAULT_MEMORY_MB: u32 = 1024;
pub const DEFAULT_CPU: f32 = 1.0;
const DEFAULT_PIDS: u32 = 256;

/// Docker's default network: the way out to the internet
pub const BRIDGE_NETWORK: &str = "bridge";
const NO_NETWORK: &str = "none";

/// What a container may use of the machine
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Limits {
    pub memory_mb: u32,
    pub cpus: f32,
    pub pids: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            memory_mb: DEFAULT_MEMORY_MB,
            cpus: DEFAULT_CPU,
            pids: DEFAULT_PIDS,
        }
    }
}

/// Whether a mounted folder or volume can be written from inside
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    ReadOnly,
    ReadWrite,
}

impl Access {
    pub fn read_only_if(read_only: bool) -> Self {
        if read_only {
            Self::ReadOnly
        } else {
            Self::ReadWrite
        }
    }
}

/// What a mount brings in: a host path or a named Docker volume
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    Bind,
    Volume,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SandboxMount {
    kind: Source,
    source: String,
    target: String,
    access: Access,
}

/// A memory-backed scratch folder: the writable places besides the mounts
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tmpfs {
    path: String,
    size_mb: u32,
    /// Owned by the container's user (a home folder), not by root
    owned: bool,
    /// Programs may run from it (Docker mounts a tmpfs `noexec` by default)
    exec: bool,
}

impl Tmpfs {
    /// A scratch folder owned by root, like `/tmp`
    pub fn scratch(path: &str, size_mb: u32) -> Self {
        Self {
            path: path.to_string(),
            size_mb,
            owned: false,
            exec: false,
        }
    }

    /// A home folder owned by the container's user
    pub fn home(path: &str, size_mb: u32) -> Self {
        Self {
            owned: true,
            ..Self::scratch(path, size_mb)
        }
    }

    /// Lets programs run from it
    pub fn exec(self) -> Self {
        Self { exec: true, ..self }
    }

    fn options(&self, user: Option<AgentUser>) -> String {
        let exec = if self.exec { "exec," } else { "" };
        let options = format!("rw,{exec}nosuid,nodev,size={}m", self.size_mb);
        match user {
            Some(user) if self.owned => user.home_tmpfs(&options),
            _ => options,
        }
    }
}

/// A sandboxed container: what it runs, as whom, and what it can reach.
/// Build it with [`SandboxSpec::new`] and the setters; see the module docs
/// for what is on from the start.
#[derive(Debug, Clone, PartialEq)]
pub struct SandboxSpec {
    image: String,
    name: Option<String>,
    labels: BTreeMap<String, String>,
    /// `None`: the image's own user, for images that aren't Nandi's
    user: Option<AgentUser>,
    read_only_root: bool,
    network: String,
    mounts: Vec<SandboxMount>,
    tmpfs: Vec<Tmpfs>,
    limits: Limits,
    shm_size_mb: Option<u32>,
    workdir: Option<String>,
    /// `NAME=value`, shown on a command line: never secrets
    env: Vec<String>,
    /// Passed by name only (`-e NAME`): Docker takes the value from its own
    /// environment, so secrets never appear in a command line
    env_names: Vec<String>,
    /// Replaces the image's entrypoint; empty clears it
    entrypoint: Option<String>,
    command: Vec<String>,
    init: bool,
    interactive: bool,
    remove_on_exit: bool,
    /// Ports published on the host's loopback only
    loopback_ports: Vec<u16>,
    /// `host.docker.internal` resolves to the host
    reaches_host: bool,
}

impl SandboxSpec {
    /// A hardened container of `image`: Nandi's image user, a read-only
    /// root, no network and the default limits
    pub fn new(image: &str) -> Self {
        Self {
            image: image.to_string(),
            name: None,
            labels: BTreeMap::new(),
            user: Some(AgentUser::IMAGE),
            read_only_root: true,
            network: NO_NETWORK.to_string(),
            mounts: Vec::new(),
            tmpfs: Vec::new(),
            limits: Limits::default(),
            shm_size_mb: None,
            workdir: None,
            env: Vec::new(),
            env_names: Vec::new(),
            entrypoint: None,
            command: Vec::new(),
            init: false,
            interactive: false,
            remove_on_exit: false,
            loopback_ports: Vec::new(),
            reaches_host: false,
        }
    }

    pub fn name(mut self, name: &str) -> Self {
        self.name = Some(name.to_string());
        self
    }

    pub fn label(mut self, key: &str, value: &str) -> Self {
        self.labels.insert(key.to_string(), value.to_string());
        self
    }

    /// Runs as `user` (never root: see [`AgentUser`])
    pub fn user(mut self, user: AgentUser) -> Self {
        self.user = Some(user);
        self
    }

    /// Runs as the image's own user: for images that aren't Nandi's
    pub fn image_user(mut self) -> Self {
        self.user = None;
        self
    }

    /// A writable root filesystem, for images that write outside their
    /// mounts and can't be told otherwise
    pub fn writable_root(mut self) -> Self {
        self.read_only_root = false;
        self
    }

    pub fn network(mut self, network: &str) -> Self {
        self.network = network.to_string();
        self
    }

    /// A host folder at `target`
    pub fn bind(mut self, source: &str, target: &str, access: Access) -> Self {
        self.mounts.push(SandboxMount {
            kind: Source::Bind,
            source: source.to_string(),
            target: target.to_string(),
            access,
        });
        self
    }

    /// A named Docker volume at `target`
    pub fn volume(mut self, volume: &str, target: &str, access: Access) -> Self {
        self.mounts.push(SandboxMount {
            kind: Source::Volume,
            source: volume.to_string(),
            target: target.to_string(),
            access,
        });
        self
    }

    pub fn tmpfs(mut self, tmpfs: Tmpfs) -> Self {
        self.tmpfs.push(tmpfs);
        self
    }

    pub fn memory_mb(mut self, memory_mb: u32) -> Self {
        self.limits.memory_mb = memory_mb;
        self
    }

    pub fn cpus(mut self, cpus: f32) -> Self {
        self.limits.cpus = cpus;
        self
    }

    pub fn pids(mut self, pids: u32) -> Self {
        self.limits.pids = pids;
        self
    }

    /// A larger `/dev/shm` (Chrome needs one)
    pub fn shm_size_mb(mut self, size_mb: u32) -> Self {
        self.shm_size_mb = Some(size_mb);
        self
    }

    pub fn workdir(mut self, workdir: &str) -> Self {
        self.workdir = Some(workdir.to_string());
        self
    }

    /// `NAME=value` variables. They show on a `docker run` command line, so
    /// never a secret: pass those with [`Self::env_names`]
    pub fn env(mut self, vars: impl IntoIterator<Item = String>) -> Self {
        self.env.extend(vars);
        self
    }

    /// Variables passed by name, their values from the `docker` process's
    /// own environment. Command lines only ([`Self::to_cli_args`])
    pub fn env_names<S: AsRef<str>>(mut self, names: impl IntoIterator<Item = S>) -> Self {
        self.env_names
            .extend(names.into_iter().map(|name| name.as_ref().to_string()));
        self
    }

    /// Replaces the image's entrypoint; `""` clears it
    pub fn entrypoint(mut self, entrypoint: &str) -> Self {
        self.entrypoint = Some(entrypoint.to_string());
        self
    }

    /// What runs, after the image's entrypoint
    pub fn command<S: AsRef<str>>(mut self, command: impl IntoIterator<Item = S>) -> Self {
        self.command
            .extend(command.into_iter().map(|arg| arg.as_ref().to_string()));
        self
    }

    /// A tiny init as PID 1, which reaps what the container leaves behind
    pub fn init(mut self) -> Self {
        self.init = true;
        self
    }

    /// Keeps stdin open: for stdio servers (`docker run -i`)
    pub fn interactive(mut self) -> Self {
        self.interactive = true;
        self
    }

    /// Docker removes the container when it exits
    pub fn remove_on_exit(mut self) -> Self {
        self.remove_on_exit = true;
        self
    }

    /// Publishes `port` on the host's `127.0.0.1` only
    pub fn loopback_port(mut self, port: u16) -> Self {
        self.loopback_ports.push(port);
        self
    }

    /// `host.docker.internal` reaches the host (Docker's `host-gateway`)
    pub fn reaches_host(mut self) -> Self {
        self.reaches_host = true;
        self
    }

    /// The `docker run` arguments, the command after the image
    pub fn to_cli_args(&self) -> Vec<String> {
        let mut args = vec!["run".to_string()];
        let mut flag = |on: bool, name: &str| {
            if on {
                args.push(name.to_string());
            }
        };
        flag(self.interactive, "-i");
        flag(self.remove_on_exit, "--rm");
        flag(self.init, "--init");
        if let Some(name) = &self.name {
            args.extend(["--name".to_string(), name.clone()]);
        }
        for (key, value) in &self.labels {
            args.extend(["--label".to_string(), format!("{key}={value}")]);
        }
        args.extend(HARDENING.iter().map(|option| format!("--{option}")));
        let Limits {
            memory_mb,
            cpus,
            pids,
        } = self.limits;
        args.extend([
            format!("--pids-limit={pids}"),
            format!("--memory={memory_mb}m"),
            format!("--cpus={cpus}"),
        ]);
        if let Some(size_mb) = self.shm_size_mb {
            args.push(format!("--shm-size={size_mb}m"));
        }
        if let Some(user) = self.user {
            args.push(format!("--user={}", user.user()));
        }
        if self.read_only_root {
            args.push("--read-only".to_string());
        }
        for tmpfs in &self.tmpfs {
            args.push(format!(
                "--tmpfs={}:{}",
                tmpfs.path,
                tmpfs.options(self.user)
            ));
        }
        args.push(format!("--network={}", self.network));
        if self.reaches_host {
            args.push(format!("--add-host={HOST_GATEWAY}"));
        }
        for port in &self.loopback_ports {
            args.push(format!("--publish=127.0.0.1:{port}:{port}"));
        }
        for mount in &self.mounts {
            args.extend(["--mount".to_string(), mount_arg(mount)]);
        }
        if let Some(workdir) = &self.workdir {
            args.extend(["-w".to_string(), workdir.clone()]);
        }
        for var in self.env.iter().chain(&self.env_names) {
            args.extend(["-e".to_string(), var.clone()]);
        }
        if let Some(entrypoint) = &self.entrypoint {
            args.push(format!("--entrypoint={entrypoint}"));
        }
        args.push(self.image.clone());
        args.extend(self.command.iter().cloned());
        args
    }

    /// The bollard create body. Bollard takes the name in its create
    /// options, not here. Variables passed by name have no value here, so a
    /// spec for bollard sets them with [`Self::env`]
    pub fn to_bollard(&self) -> ContainerCreateBody {
        debug_assert!(
            self.env_names.is_empty(),
            "bollard has no environment to take {:?} from",
            self.env_names
        );
        let Limits {
            memory_mb,
            cpus,
            pids,
        } = self.limits;
        let some_vec = |items: Vec<String>| (!items.is_empty()).then_some(items);
        let port = |port: &u16| format!("{port}/tcp");
        ContainerCreateBody {
            image: Some(self.image.clone()),
            user: self.user.map(AgentUser::user),
            labels: (!self.labels.is_empty()).then(|| {
                self.labels
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect()
            }),
            env: some_vec(self.env.clone()),
            working_dir: self.workdir.clone(),
            entrypoint: self.entrypoint.as_ref().map(|entrypoint| {
                if entrypoint.is_empty() {
                    Vec::new()
                } else {
                    vec![entrypoint.clone()]
                }
            }),
            cmd: some_vec(self.command.clone()),
            open_stdin: self.interactive.then_some(true),
            exposed_ports: some_vec(self.loopback_ports.iter().map(port).collect()),
            host_config: Some(HostConfig {
                network_mode: Some(self.network.clone()),
                init: self.init.then_some(true),
                auto_remove: self.remove_on_exit.then_some(true),
                mounts: (!self.mounts.is_empty())
                    .then(|| self.mounts.iter().map(bollard_mount).collect()),
                readonly_rootfs: self.read_only_root.then_some(true),
                tmpfs: (!self.tmpfs.is_empty()).then(|| {
                    self.tmpfs
                        .iter()
                        .map(|tmpfs| (tmpfs.path.clone(), tmpfs.options(self.user)))
                        .collect()
                }),
                cap_drop: Some(vec!["ALL".to_string()]),
                security_opt: Some(vec![NO_NEW_PRIVILEGES.to_string()]),
                memory: Some(i64::from(memory_mb) * 1024 * 1024),
                nano_cpus: Some(nano_cpus(cpus)),
                pids_limit: Some(i64::from(pids)),
                shm_size: self
                    .shm_size_mb
                    .map(|size_mb| i64::from(size_mb) * 1024 * 1024),
                extra_hosts: self.reaches_host.then(|| vec![HOST_GATEWAY.to_string()]),
                port_bindings: (!self.loopback_ports.is_empty()).then(|| {
                    self.loopback_ports
                        .iter()
                        .map(|loopback| {
                            let binding = PortBinding {
                                host_ip: Some("127.0.0.1".to_string()),
                                host_port: Some(loopback.to_string()),
                            };
                            (port(loopback), Some(vec![binding]))
                        })
                        .collect::<HashMap<_, _>>()
                }),
                ..Default::default()
            }),
            ..Default::default()
        }
    }
}

const NO_NEW_PRIVILEGES: &str = "no-new-privileges";
/// The `docker run` options every container gets
const HARDENING: [&str; 2] = ["cap-drop=ALL", "security-opt=no-new-privileges"];
const HOST_GATEWAY: &str = "host.docker.internal:host-gateway";

/// Docker's CPU quota, in billionths of a core
fn nano_cpus(cpus: f32) -> i64 {
    (f64::from(cpus) * 1e9).round() as i64
}

/// `--mount`'s value. Docker reads it as one CSV record, so a field with a
/// comma or a quote is quoted: a path can't add mount options
fn mount_arg(mount: &SandboxMount) -> String {
    let kind = match mount.kind {
        Source::Bind => "bind",
        Source::Volume => "volume",
    };
    let mut fields = vec![
        format!("type={kind}"),
        csv_field(&format!("source={}", mount.source)),
        csv_field(&format!("target={}", mount.target)),
    ];
    if mount.access == Access::ReadOnly {
        fields.push("readonly".to_string());
    }
    fields.join(",")
}

fn csv_field(field: &str) -> String {
    if field.contains([',', '"']) {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field.to_string()
    }
}

fn bollard_mount(mount: &SandboxMount) -> Mount {
    Mount {
        typ: Some(match mount.kind {
            Source::Bind => MountType::BIND,
            Source::Volume => MountType::VOLUME,
        }),
        source: Some(mount.source.clone()),
        target: Some(mount.target.clone()),
        read_only: Some(mount.access == Access::ReadOnly),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const IMAGE: &str = "ghcr.io/vaultdbai/nandi-runtime:sandbox-6";

    fn has_pair(args: &[String], flag: &str, value: &str) -> bool {
        args.windows(2)
            .any(|pair| pair[0] == flag && pair[1] == value)
    }

    #[test]
    fn every_container_starts_hardened_and_limited() {
        let spec = SandboxSpec::new(IMAGE);
        let args = spec.to_cli_args();
        for flag in [
            "--cap-drop=ALL",
            "--security-opt=no-new-privileges",
            "--read-only",
            "--user=1000:1000",
            "--network=none",
            "--pids-limit=256",
            "--memory=1024m",
            "--cpus=1",
        ] {
            assert!(args.contains(&flag.to_string()), "{flag}: {args:?}");
        }
        assert_eq!(args.first().map(String::as_str), Some("run"));
        assert_eq!(args.last().map(String::as_str), Some(IMAGE));

        let body = spec.to_bollard();
        let host = body.host_config.unwrap();
        assert_eq!(body.user.as_deref(), Some("1000:1000"));
        assert_eq!(host.cap_drop, Some(vec!["ALL".to_string()]));
        assert_eq!(
            host.security_opt,
            Some(vec!["no-new-privileges".to_string()])
        );
        assert_eq!(host.readonly_rootfs, Some(true));
        assert_eq!(host.network_mode.as_deref(), Some("none"));
        assert_eq!(host.memory, Some(1024 * 1024 * 1024));
        assert_eq!(host.nano_cpus, Some(1_000_000_000));
        assert_eq!(host.pids_limit, Some(256));
        assert!(host.port_bindings.is_none() && body.exposed_ports.is_none());
        assert!(host.extra_hosts.is_none() && body.env.is_none());
    }

    #[test]
    fn opting_out_keeps_the_hardening_that_is_always_on() {
        let spec = SandboxSpec::new("mcr.microsoft.com/playwright/mcp")
            .image_user()
            .writable_root();
        let args = spec.to_cli_args();
        assert!(!args.iter().any(|arg| arg.starts_with("--user")));
        assert!(!args.contains(&"--read-only".to_string()));
        assert!(args.contains(&"--cap-drop=ALL".to_string()));
        assert!(args.contains(&"--security-opt=no-new-privileges".to_string()));

        let body = spec.to_bollard();
        let host = body.host_config.unwrap();
        assert_eq!(body.user, None);
        assert_eq!(host.readonly_rootfs, None);
        assert_eq!(host.cap_drop, Some(vec!["ALL".to_string()]));
    }

    #[test]
    fn mounts_render_as_mount_flags_and_bollard_mounts() {
        struct Case {
            spec: SandboxSpec,
            cli: &'static str,
            kind: MountType,
            read_only: bool,
        }
        let cases = [
            Case {
                spec: SandboxSpec::new(IMAGE).bind(
                    "/home/a/docs",
                    "/agent/folders/docs",
                    Access::ReadOnly,
                ),
                cli: "type=bind,source=/home/a/docs,target=/agent/folders/docs,readonly",
                kind: MountType::BIND,
                read_only: true,
            },
            Case {
                spec: SandboxSpec::new(IMAGE).bind("/work", "/agent/workdir", Access::ReadWrite),
                cli: "type=bind,source=/work,target=/agent/workdir",
                kind: MountType::BIND,
                read_only: false,
            },
            Case {
                spec: SandboxSpec::new(IMAGE).volume("nandi-tools", "/opt/tools", Access::ReadOnly),
                cli: "type=volume,source=nandi-tools,target=/opt/tools,readonly",
                kind: MountType::VOLUME,
                read_only: true,
            },
            // A colon is only a character; a comma or a quote can't add options
            Case {
                spec: SandboxSpec::new(IMAGE).bind(
                    r"C:\a:b",
                    "/agent/folders/x",
                    Access::ReadWrite,
                ),
                cli: r"type=bind,source=C:\a:b,target=/agent/folders/x",
                kind: MountType::BIND,
                read_only: false,
            },
            Case {
                spec: SandboxSpec::new(IMAGE).bind("/a,readonly=false\"", "/t", Access::ReadOnly),
                cli: "type=bind,\"source=/a,readonly=false\"\"\",target=/t,readonly",
                kind: MountType::BIND,
                read_only: true,
            },
        ];
        for case in cases {
            let args = case.spec.to_cli_args();
            assert!(
                has_pair(&args, "--mount", case.cli),
                "{}: {args:?}",
                case.cli
            );
            assert!(!args.contains(&"-v".to_string()));
            let host = case.spec.to_bollard().host_config.unwrap();
            let mount = &host.mounts.unwrap()[0];
            assert_eq!(mount.typ, Some(case.kind), "{}", case.cli);
            assert_eq!(mount.read_only, Some(case.read_only), "{}", case.cli);
        }
    }

    #[test]
    fn tmpfs_options_follow_their_kind_and_the_user() {
        let user = AgentUser {
            uid: 1001,
            gid: 118,
        };
        let cases = [
            (
                Tmpfs::scratch("/tmp", 512),
                Some(user),
                "rw,nosuid,nodev,size=512m",
            ),
            (
                Tmpfs::home("/home/node", 64),
                Some(user),
                "rw,nosuid,nodev,size=64m,uid=1001,gid=118",
            ),
            (
                Tmpfs::home("/home/node", 512).exec(),
                Some(AgentUser::IMAGE),
                "rw,exec,nosuid,nodev,size=512m,uid=1000,gid=1000",
            ),
            // The image's own user: Docker's default owner
            (Tmpfs::home("/home/x", 64), None, "rw,nosuid,nodev,size=64m"),
        ];
        for (tmpfs, user, options) in cases {
            let mut spec = SandboxSpec::new(IMAGE).tmpfs(tmpfs.clone());
            spec = match user {
                Some(user) => spec.user(user),
                None => spec.image_user(),
            };
            let args = spec.to_cli_args();
            let flag = format!("--tmpfs={}:{options}", tmpfs.path);
            assert!(args.contains(&flag), "{flag}: {args:?}");
            let host = spec.to_bollard().host_config.unwrap();
            assert_eq!(host.tmpfs.unwrap()[&tmpfs.path], options);
        }
    }

    #[test]
    fn limits_render_for_both_forms() {
        let cases = [
            (
                Limits::default(),
                "--memory=1024m",
                "--cpus=1",
                1_000_000_000,
            ),
            (
                Limits {
                    memory_mb: 2048,
                    cpus: 1.5,
                    pids: 512,
                },
                "--memory=2048m",
                "--cpus=1.5",
                1_500_000_000,
            ),
            (
                Limits {
                    memory_mb: 64,
                    cpus: 0.25,
                    pids: 64,
                },
                "--memory=64m",
                "--cpus=0.25",
                250_000_000,
            ),
        ];
        for (limits, memory, cpus, nano) in cases {
            let spec = SandboxSpec::new(IMAGE)
                .memory_mb(limits.memory_mb)
                .cpus(limits.cpus)
                .pids(limits.pids);
            let args = spec.to_cli_args();
            assert!(args.contains(&memory.to_string()), "{args:?}");
            assert!(args.contains(&cpus.to_string()), "{args:?}");
            assert!(args.contains(&format!("--pids-limit={}", limits.pids)));
            let host = spec.to_bollard().host_config.unwrap();
            assert_eq!(host.memory, Some(i64::from(limits.memory_mb) * 1024 * 1024));
            assert_eq!(host.nano_cpus, Some(nano));
            assert_eq!(host.pids_limit, Some(i64::from(limits.pids)));
        }
    }

    #[test]
    fn secrets_pass_by_name_and_only_plain_values_are_shown() {
        let args = SandboxSpec::new(IMAGE)
            .env(AgentUser::env("/home/node", "node"))
            .env_names(["ANTHROPIC_API_KEY", "AWS_SESSION_TOKEN"])
            .to_cli_args();
        let passed: Vec<&str> = args
            .windows(2)
            .filter(|pair| pair[0] == "-e")
            .map(|pair| pair[1].as_str())
            .collect();
        assert_eq!(
            passed,
            [
                "HOME=/home/node",
                "USER=node",
                "ANTHROPIC_API_KEY",
                "AWS_SESSION_TOKEN"
            ]
        );
    }

    #[test]
    fn the_command_follows_the_image_and_the_entrypoint_comes_before() {
        let spec = SandboxSpec::new(IMAGE)
            .name("nandi-connector-github")
            .label("nandi.connector", "github")
            .interactive()
            .remove_on_exit()
            .init()
            .workdir("/agent/workdir")
            .entrypoint("npx")
            .command(["-y", "@acme/github-mcp"]);
        let args = spec.to_cli_args();
        assert_eq!(
            args[..6],
            [
                "run",
                "-i",
                "--rm",
                "--init",
                "--name",
                "nandi-connector-github"
            ]
        );
        assert!(has_pair(&args, "--label", "nandi.connector=github"));
        assert!(has_pair(&args, "-w", "/agent/workdir"));
        assert!(args.ends_with(&[
            "--entrypoint=npx".to_string(),
            IMAGE.to_string(),
            "-y".to_string(),
            "@acme/github-mcp".to_string(),
        ]));

        let body = spec.to_bollard();
        assert_eq!(body.entrypoint, Some(vec!["npx".to_string()]));
        assert_eq!(
            body.cmd,
            Some(vec!["-y".to_string(), "@acme/github-mcp".to_string()])
        );
        assert_eq!(body.labels.unwrap()["nandi.connector"], "github");
        assert_eq!(body.open_stdin, Some(true));
        let host = body.host_config.unwrap();
        assert_eq!((host.init, host.auto_remove), (Some(true), Some(true)));
        // An empty entrypoint clears the image's
        let cleared = SandboxSpec::new(IMAGE).entrypoint("").to_bollard();
        assert_eq!(cleared.entrypoint, Some(Vec::new()));
    }

    #[test]
    fn host_access_is_loopback_only_and_explicit() {
        let spec = SandboxSpec::new(IMAGE)
            .network(BRIDGE_NETWORK)
            .reaches_host()
            .loopback_port(4000);
        let args = spec.to_cli_args();
        assert!(args.contains(&"--network=bridge".to_string()));
        assert!(args.contains(&"--add-host=host.docker.internal:host-gateway".to_string()));
        assert!(args.contains(&"--publish=127.0.0.1:4000:4000".to_string()));

        let body = spec.to_bollard();
        assert_eq!(body.exposed_ports, Some(vec!["4000/tcp".to_string()]));
        let host = body.host_config.unwrap();
        let binding = &host.port_bindings.unwrap()["4000/tcp"].clone().unwrap()[0];
        assert_eq!(binding.host_ip.as_deref(), Some("127.0.0.1"));
        assert_eq!(binding.host_port.as_deref(), Some("4000"));
        assert_eq!(
            host.extra_hosts,
            Some(vec!["host.docker.internal:host-gateway".to_string()])
        );
    }
}
