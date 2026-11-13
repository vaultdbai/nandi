//! The LiteLLM gateway: an Anthropic-Messages proxy in front of Bedrock,
//! so Claude Code can drive third-party Bedrock models. It is the official
//! LiteLLM image, unchanged; Nandi only renders its config (from
//! `runtimes/nandi-harness/litellm-config-template.yaml`) and runs it:
//!
//! - container `nandi-litellm` on the `nandi-harness` Docker network (its way
//!   to Bedrock) and on the internal `nandi-agents` one, so coding agents
//!   reach it directly at `http://nandi-litellm:4000`; published on
//!   `127.0.0.1:4000` for its health check
//! - AWS credentials and a per-start master key go in as container
//!   environment (Docker API body, never a file or a command line); the
//!   rendered config holds neither
//! - one gateway, one model at a time

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use bollard::models::{
    ContainerCreateBody, HostConfig, Mount, MountType, NetworkCreateRequest, PortBinding,
};
use bollard::query_parameters::{CreateContainerOptions, RemoveContainerOptions};
use bollard::Docker;
use serde::{Deserialize, Serialize};
use specta::Type;

use super::adapter::{GatewayConfig, GatewayKind, HarnessCredentials};
use crate::db::{now_ms, ts};
use crate::model::Timestamp;
use crate::services::events::{self, EventBus, NandiEvent};
use crate::services::AppState;

/// The official image, pinned (released 2026-09)
pub const LITELLM_IMAGE: &str = "ghcr.io/berriai/litellm:v1.102.0";
pub const CONTAINER_NAME: &str = "nandi-litellm";
/// The Docker network harness containers join to reach the gateway
pub const NETWORK: &str = "nandi-harness";
pub const PORT: u16 = 4000;
/// The model name Claude Code asks the gateway for
pub const MODEL_ALIAS: &str = "nandi-bedrock";
pub const SMALL_MODEL_ALIAS: &str = "nandi-bedrock-small";
const TEMPLATE: &str =
    include_str!("../../../../runtimes/nandi-harness/litellm-config-template.yaml");
/// Inside the container
const CONFIG_PATH: &str = "/app/nandi-litellm.yaml";
const READY_TIMEOUT: Duration = Duration::from_secs(90);

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LitellmGatewayStatus {
    pub running: bool,
    /// For host sessions, e.g. `http://127.0.0.1:4000`
    pub endpoint: Option<String>,
    /// The Bedrock model it serves
    pub model: Option<String>,
    pub container_id: Option<String>,
    pub started_at: Option<Timestamp>,
    pub error: Option<String>,
}

impl NandiEvent for LitellmGatewayStatus {
    const NAME: &'static str = super::BEDROCK_GATEWAY_STATUS;
}

/// The running gateway, as this app run started it
struct Running {
    model: String,
    master_key: String,
    container_id: String,
    started_at: Timestamp,
}

pub struct GatewayService {
    bus: EventBus,
    config_dir: PathBuf,
    running: Mutex<Option<Running>>,
}

/// Bedrock model IDs are `[A-Za-z0-9._:/-]`; anything else could break
/// out of the YAML string it's put in
pub fn valid_model_id(model: &str) -> bool {
    !model.is_empty()
        && model.len() <= 256
        && model
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._:/-".contains(c))
}

/// The LiteLLM config for `model` (no secrets: those are env references)
pub fn render_config(model: &str) -> Result<String, String> {
    if !valid_model_id(model) {
        return Err(format!("'{model}' is not a Bedrock model ID"));
    }
    let model = model.strip_prefix("bedrock/").unwrap_or(model);
    let rendered = TEMPLATE
        .replace("{{MODEL_ALIAS}}", MODEL_ALIAS)
        .replace("{{SMALL_MODEL_ALIAS}}", SMALL_MODEL_ALIAS)
        .replace("{{BEDROCK_MODEL}}", model);
    // Must still be YAML LiteLLM can read
    serde_yaml::from_str::<serde_yaml::Value>(&rendered).map_err(|e| e.to_string())?;
    Ok(rendered)
}

/// The container's environment: AWS credentials and the master key
fn container_env(credentials: &HarnessCredentials, master_key: &str) -> Vec<String> {
    let mut env = vec![format!("LITELLM_MASTER_KEY={master_key}")];
    for (name, value) in super::adapter::common_credential_env(credentials) {
        env.push(format!("{name}={value}"));
    }
    if credentials.aws_region.is_none() {
        env.push("AWS_REGION=us-east-1".to_string());
    }
    env
}

fn docker() -> Result<Docker, String> {
    Docker::connect_with_local_defaults().map_err(|e| format!("Docker is not available: {e}"))
}

/// Creates the `nandi-harness` network if it's missing
pub async fn ensure_network() -> Result<(), String> {
    let docker = docker()?;
    if docker.inspect_network(NETWORK, None).await.is_ok() {
        return Ok(());
    }
    let request = NetworkCreateRequest {
        name: NETWORK.to_string(),
        driver: Some("bridge".to_string()),
        labels: Some(HashMap::from([(
            "nandi.harness".to_string(),
            "1".to_string(),
        )])),
        ..Default::default()
    };
    match docker.create_network(request).await {
        Ok(_) => Ok(()),
        // Created concurrently
        Err(_) if docker.inspect_network(NETWORK, None).await.is_ok() => Ok(()),
        Err(e) => Err(format!("Could not create the {NETWORK} network: {e}")),
    }
}

impl GatewayService {
    pub fn new(bus: EventBus, config_dir: PathBuf) -> Self {
        Self {
            bus,
            config_dir,
            running: Mutex::new(None),
        }
    }

    /// Where a harness reaches the gateway, with the key it must present;
    /// `None` when it isn't running
    pub fn session_gateway(&self, in_container: bool) -> Option<(GatewayConfig, String)> {
        let running = self.running.lock().ok()?;
        let running = running.as_ref()?;
        let endpoint = if in_container {
            format!("http://{CONTAINER_NAME}:{PORT}")
        } else {
            format!("http://127.0.0.1:{PORT}")
        };
        Some((
            GatewayConfig {
                kind: GatewayKind::Litellm,
                endpoint: Some(endpoint),
            },
            running.master_key.clone(),
        ))
    }

    /// The Bedrock model the gateway serves, if it runs
    pub fn model(&self) -> Option<String> {
        self.running.lock().ok()?.as_ref().map(|r| r.model.clone())
    }

    /// (Re)starts the gateway for `model` with `credentials`; returns once
    /// it answers its health check
    pub async fn start(
        &self,
        model: &str,
        credentials: &HarnessCredentials,
    ) -> Result<LitellmGatewayStatus, String> {
        let config = render_config(model)?;
        tokio::fs::create_dir_all(&self.config_dir)
            .await
            .map_err(|e| e.to_string())?;
        let config_file = self.config_dir.join("litellm.yaml");
        tokio::fs::write(&config_file, config)
            .await
            .map_err(|e| format!("Could not write the gateway config: {e}"))?;

        let docker = docker()?;
        if docker.inspect_image(LITELLM_IMAGE).await.is_err() {
            return Err(format!(
                "The LiteLLM image is missing. Pull it with: docker pull {LITELLM_IMAGE}"
            ));
        }
        ensure_network().await?;
        crate::services::egress::relay::ensure_agent_network().await?;
        remove(&docker).await;
        let master_key = format!("sk-nandi-{}", uuid::Uuid::new_v4().simple());
        let body = ContainerCreateBody {
            image: Some(LITELLM_IMAGE.to_string()),
            cmd: Some(vec![
                "--config".to_string(),
                CONFIG_PATH.to_string(),
                "--port".to_string(),
                PORT.to_string(),
            ]),
            env: Some(container_env(credentials, &master_key)),
            labels: Some(HashMap::from([
                ("nandi.harness".to_string(), "1".to_string()),
                ("nandi.gateway.model".to_string(), model.to_string()),
            ])),
            exposed_ports: Some(vec![format!("{PORT}/tcp")]),
            host_config: Some(HostConfig {
                mounts: Some(vec![Mount {
                    typ: Some(MountType::BIND),
                    source: Some(config_file.display().to_string()),
                    target: Some(CONFIG_PATH.to_string()),
                    read_only: Some(true),
                    ..Default::default()
                }]),
                port_bindings: Some(HashMap::from([(
                    format!("{PORT}/tcp"),
                    Some(vec![PortBinding {
                        host_ip: Some("127.0.0.1".to_string()),
                        host_port: Some(PORT.to_string()),
                    }]),
                )])),
                network_mode: Some(NETWORK.to_string()),
                security_opt: Some(vec!["no-new-privileges".to_string()]),
                ..Default::default()
            }),
            ..Default::default()
        };
        let options = CreateContainerOptions {
            name: Some(CONTAINER_NAME.to_string()),
            ..Default::default()
        };
        let created = docker
            .create_container(Some(options), body)
            .await
            .map_err(|e| format!("Could not create the gateway container: {e}"))?;
        // Coding agents only have the internal network; join it
        let joined = docker
            .connect_network(
                crate::services::egress::relay::AGENT_NETWORK,
                bollard::models::NetworkConnectRequest {
                    container: created.id.clone(),
                    ..Default::default()
                },
            )
            .await;
        if let Err(e) = joined {
            remove(&docker).await;
            return Err(format!(
                "Could not attach the gateway to the agents' network: {e}"
            ));
        }
        if let Err(e) = docker.start_container(&created.id, None).await {
            remove(&docker).await;
            return Err(format!("Could not start the gateway: {e}"));
        }
        if let Err(e) = wait_ready().await {
            remove(&docker).await;
            return Err(e);
        }
        let started_at = ts(now_ms());
        if let Ok(mut running) = self.running.lock() {
            *running = Some(Running {
                model: model.to_string(),
                master_key,
                container_id: created.id,
                started_at,
            });
        }
        let status = self.status().await;
        events::emit(&self.bus, &status);
        Ok(status)
    }

    pub async fn stop(&self) -> Result<LitellmGatewayStatus, String> {
        if let Ok(mut running) = self.running.lock() {
            *running = None;
        }
        remove(&docker()?).await;
        let status = self.status().await;
        events::emit(&self.bus, &status);
        Ok(status)
    }

    /// What this app run started, checked against Docker
    pub async fn status(&self) -> LitellmGatewayStatus {
        let known = self.running.lock().ok().and_then(|r| {
            r.as_ref()
                .map(|r| (r.model.clone(), r.container_id.clone(), r.started_at))
        });
        let Some((model, container_id, started_at)) = known else {
            return LitellmGatewayStatus::default();
        };
        let running = match crate::runtime::container_running(&container_id).await {
            Ok(running) => running,
            Err(e) => {
                return LitellmGatewayStatus {
                    error: Some(e),
                    ..LitellmGatewayStatus::default()
                }
            }
        };
        LitellmGatewayStatus {
            running,
            endpoint: running.then(|| format!("http://127.0.0.1:{PORT}")),
            model: Some(model),
            container_id: Some(container_id),
            started_at: Some(started_at),
            error: (!running).then(|| "The gateway container stopped".to_string()),
        }
    }
}

async fn remove(docker: &Docker) {
    let options = RemoveContainerOptions {
        force: true,
        ..Default::default()
    };
    let _ = docker.remove_container(CONTAINER_NAME, Some(options)).await;
}

/// Polls LiteLLM's liveness endpoint until it answers
async fn wait_ready() -> Result<(), String> {
    let client = reqwest::Client::new();
    let url = format!("http://127.0.0.1:{PORT}/health/liveliness");
    let deadline = tokio::time::Instant::now() + READY_TIMEOUT;
    while tokio::time::Instant::now() < deadline {
        if let Ok(response) = client
            .get(&url)
            .timeout(Duration::from_secs(2))
            .send()
            .await
        {
            if response.status().is_success() {
                return Ok(());
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    Err(format!(
        "The gateway did not become ready within {}s",
        READY_TIMEOUT.as_secs()
    ))
}

/// Starts the gateway with the AWS identity Bedrock sessions use
pub async fn start_litellm_gateway(
    state: &AppState,
    model: &str,
) -> Result<LitellmGatewayStatus, String> {
    let credentials = super::aws_auth::bedrock_credentials(state, None, None).await?;
    state.litellm_gateway().start(model, &credentials).await
}

pub async fn stop_litellm_gateway(state: &AppState) -> Result<LitellmGatewayStatus, String> {
    state.litellm_gateway().stop().await
}

pub async fn get_litellm_gateway_status(state: &AppState) -> Result<LitellmGatewayStatus, String> {
    Ok(state.litellm_gateway().status().await)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_template_renders_for_a_bedrock_model() {
        let rendered = render_config("meta.llama3-3-70b-instruct-v1:0").unwrap();
        let yaml: serde_yaml::Value = serde_yaml::from_str(&rendered).unwrap();
        let models = yaml["model_list"].as_sequence().unwrap();
        assert_eq!(models.len(), 2);
        assert_eq!(models[0]["model_name"], MODEL_ALIAS);
        assert_eq!(
            models[0]["litellm_params"]["model"],
            "bedrock/meta.llama3-3-70b-instruct-v1:0"
        );
        assert_eq!(
            models[0]["litellm_params"]["aws_region_name"],
            "os.environ/AWS_REGION"
        );
        assert_eq!(
            yaml["general_settings"]["master_key"],
            "os.environ/LITELLM_MASTER_KEY"
        );
        assert!(!rendered.contains("{{"), "every placeholder filled");
        // An already-prefixed ID isn't doubled
        let prefixed: serde_yaml::Value =
            serde_yaml::from_str(&render_config("bedrock/us.deepseek.r1-v1:0").unwrap()).unwrap();
        assert_eq!(
            prefixed["model_list"][0]["litellm_params"]["model"],
            "bedrock/us.deepseek.r1-v1:0"
        );
    }

    #[test]
    fn model_ids_cannot_inject_yaml() {
        for bad in ["", "x\"\nmaster_key: 1", "a b", "model'"] {
            assert!(render_config(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn credentials_are_env_not_config() {
        let credentials = HarnessCredentials {
            aws_access_key_id: Some("AKIA".into()),
            aws_secret_access_key: Some("secret".into()),
            aws_region: Some("eu-central-1".into()),
            ..HarnessCredentials::default()
        };
        let env = container_env(&credentials, "sk-nandi-x");
        assert!(env.contains(&"AWS_SECRET_ACCESS_KEY=secret".to_string()));
        assert!(env.contains(&"AWS_REGION=eu-central-1".to_string()));
        assert!(env.contains(&"LITELLM_MASTER_KEY=sk-nandi-x".to_string()));
        let rendered = render_config("amazon.nova-pro-v1:0").unwrap();
        assert!(!rendered.contains("secret"));
    }

    #[tokio::test]
    async fn not_running_until_started() {
        let (bus, _) = crate::services::events::tests::bus();
        let service = GatewayService::new(bus, std::env::temp_dir());
        assert_eq!(service.status().await, LitellmGatewayStatus::default());
        assert_eq!(service.session_gateway(true), None);
        assert_eq!(service.model(), None);
    }
}
