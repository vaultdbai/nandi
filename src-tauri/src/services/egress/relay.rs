//! The agents' network and its one way out.
//!
//! Every agent container (Nandi's tool sandbox and the coding agents' one)
//! joins only `nandi-agents`, a Docker network created `--internal`: no
//! route to the internet or to the host. The `nandi-egress` relay sits on
//! that network and on Docker's default bridge, and forwards whatever
//! reaches it to the host egress proxy. The proxy authenticates each
//! request by the agent's proxy token and enforces its skill.md, so a
//! process that ignores `HTTPS_PROXY` simply can't get out.
//!
//! ```text
//! agent ──nandi-agents──▶ nandi-egress:11435 ──bridge──▶ host.docker.internal:<proxy port>
//! ```
//!
//! The relay runs from whichever sandbox image is starting an agent: a
//! Python script in `nandi-runtime`, a Node one in `nandi-harness`.

use std::collections::HashMap;

use bollard::models::{
    ContainerCreateBody, HostConfig, NetworkConnectRequest, NetworkCreateRequest,
};
use bollard::query_parameters::{CreateContainerOptions, RemoveContainerOptions};
use bollard::Docker;

/// The internal network agents join
pub const AGENT_NETWORK: &str = "nandi-agents";
/// The relay container, as agents reach it on [`AGENT_NETWORK`]
pub const RELAY_NAME: &str = "nandi-egress";
/// Where the relay listens on [`AGENT_NETWORK`]
pub const RELAY_PORT: u16 = 11435;
/// Names the host proxy port the relay forwards to, so a relay left from
/// a run whose proxy took another port is replaced
const TARGET_LABEL: &str = "org.nandi.egress.target";
const RELAY_MEMORY_BYTES: i64 = 64 * 1024 * 1024;

/// How a sandbox image runs the relay
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelayImage {
    /// `nandi-runtime`: Python
    Runtime,
    /// `nandi-harness`: Node
    Harness,
}

impl RelayImage {
    fn image(self) -> &'static str {
        match self {
            Self::Runtime => crate::runtime::DEFAULT_RUNTIME_IMAGE,
            Self::Harness => crate::services::harness::HARNESS_IMAGE,
        }
    }

    fn command(self, target_port: u16) -> Vec<String> {
        let (interpreter, script) = match self {
            Self::Runtime => ("python", "/opt/nandi/relay.py"),
            Self::Harness => ("node", "/opt/nandi/relay.mjs"),
        };
        [
            interpreter,
            script,
            &RELAY_PORT.to_string(),
            "host.docker.internal",
            &target_port.to_string(),
        ]
        .iter()
        .map(|s| s.to_string())
        .collect()
    }
}

fn docker() -> Result<Docker, String> {
    Docker::connect_with_local_defaults().map_err(|e| format!("Docker is not available: {e}"))
}

/// Creates [`AGENT_NETWORK`] (internal) and starts the relay if either is
/// missing. Agents must not start without both: they'd have no way out,
/// or (on a non-internal network) one that skips the proxy.
pub async fn ensure_ready(image: RelayImage) -> Result<(), String> {
    let docker = docker()?;
    ensure_network(&docker).await?;
    ensure_relay(&docker, image).await
}

/// Creates [`AGENT_NETWORK`] if it's missing (for the LiteLLM gateway,
/// which joins it so coding agents reach it directly)
pub async fn ensure_agent_network() -> Result<(), String> {
    ensure_network(&docker()?).await
}

async fn ensure_network(docker: &Docker) -> Result<(), String> {
    if let Ok(network) = docker.inspect_network(AGENT_NETWORK, None).await {
        // A same-named network that isn't internal would let agents out
        return if network.internal == Some(true) {
            Ok(())
        } else {
            Err(format!(
                "The Docker network {AGENT_NETWORK} exists but isn't internal. Remove it \
                 (docker network rm {AGENT_NETWORK}) and Nandi recreates it."
            ))
        };
    }
    let request = NetworkCreateRequest {
        name: AGENT_NETWORK.to_string(),
        driver: Some("bridge".to_string()),
        internal: Some(true),
        labels: Some(HashMap::from([(
            "nandi.agents".to_string(),
            "1".to_string(),
        )])),
        ..Default::default()
    };
    match docker.create_network(request).await {
        Ok(_) => Ok(()),
        // Created concurrently
        Err(_) if docker.inspect_network(AGENT_NETWORK, None).await.is_ok() => Ok(()),
        Err(e) => Err(format!("Could not create the {AGENT_NETWORK} network: {e}")),
    }
}

async fn ensure_relay(docker: &Docker, image: RelayImage) -> Result<(), String> {
    let target = super::proxy_port();
    if let Ok(info) = docker.inspect_container(RELAY_NAME, None).await {
        let running = info.state.and_then(|s| s.running).unwrap_or(false);
        let same_target = info
            .config
            .and_then(|c| c.labels)
            .and_then(|labels| labels.get(TARGET_LABEL).cloned())
            == Some(target.to_string());
        let on_network = info
            .network_settings
            .and_then(|s| s.networks)
            .is_some_and(|networks| networks.contains_key(AGENT_NETWORK));
        if running && same_target && on_network {
            return Ok(());
        }
    }
    remove(docker).await;
    let created = docker
        .create_container(
            Some(CreateContainerOptions {
                name: Some(RELAY_NAME.to_string()),
                ..Default::default()
            }),
            relay_config(image, target),
        )
        .await
        .map_err(|e| format!("Could not create the egress relay: {e}"))?;
    let started = async {
        docker
            .connect_network(
                AGENT_NETWORK,
                NetworkConnectRequest {
                    container: created.id.clone(),
                    ..Default::default()
                },
            )
            .await
            .map_err(|e| format!("Could not attach the egress relay: {e}"))?;
        docker
            .start_container(&created.id, None)
            .await
            .map_err(|e| format!("Could not start the egress relay: {e}"))
    }
    .await;
    if started.is_err() {
        remove(docker).await;
    }
    started?;
    log::info!("Started the egress relay ({RELAY_NAME} → host port {target})");
    Ok(())
}

fn relay_config(image: RelayImage, target_port: u16) -> ContainerCreateBody {
    ContainerCreateBody {
        image: Some(image.image().to_string()),
        // Past the harness image's entrypoint (it only sets up Codex)
        entrypoint: Some(Vec::new()),
        cmd: Some(image.command(target_port)),
        user: Some(crate::runtime::CONTAINER_USER.to_string()),
        labels: Some(HashMap::from([
            ("nandi.egress".to_string(), "1".to_string()),
            (TARGET_LABEL.to_string(), target_port.to_string()),
        ])),
        host_config: Some(HostConfig {
            // The default bridge: its way to the host
            network_mode: Some("bridge".to_string()),
            extra_hosts: Some(vec!["host.docker.internal:host-gateway".to_string()]),
            readonly_rootfs: Some(true),
            cap_drop: Some(vec!["ALL".to_string()]),
            security_opt: Some(vec!["no-new-privileges".to_string()]),
            memory: Some(RELAY_MEMORY_BYTES),
            pids_limit: Some(64),
            init: Some(true),
            ..Default::default()
        }),
        ..Default::default()
    }
}

async fn remove(docker: &Docker) {
    let options = RemoveContainerOptions {
        force: true,
        ..Default::default()
    };
    let _ = docker.remove_container(RELAY_NAME, Some(options)).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_relay_forwards_to_the_host_proxy_and_holds_nothing() {
        let config = relay_config(RelayImage::Harness, 12345);
        assert_eq!(
            config.cmd.as_deref().unwrap(),
            [
                "node",
                "/opt/nandi/relay.mjs",
                "11435",
                "host.docker.internal",
                "12345"
            ]
        );
        assert_eq!(config.entrypoint.as_deref(), Some(&[][..]));
        assert_eq!(config.labels.as_ref().unwrap()[TARGET_LABEL], "12345");
        assert!(config.env.is_none(), "no secrets or tokens");
        let host = config.host_config.unwrap();
        assert_eq!(host.network_mode.as_deref(), Some("bridge"));
        assert_eq!(host.readonly_rootfs, Some(true));
        assert_eq!(host.cap_drop.as_deref(), Some(&["ALL".to_string()][..]));

        let python = relay_config(RelayImage::Runtime, PROXY);
        assert_eq!(python.cmd.unwrap()[..2], ["python", "/opt/nandi/relay.py"]);
    }

    const PROXY: u16 = super::super::PROXY_PORT;
}
