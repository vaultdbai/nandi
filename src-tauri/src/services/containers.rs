//! The containers on the local Docker daemon, through bollard.

use bollard::models::ContainerSummary;
use bollard::query_parameters::ListContainersOptions;
use bollard::Docker;
use serde::Serialize;
use specta::Type;

/// Summary of a Docker container, as shown in the UI.
#[derive(Debug, Clone, Serialize, Type)]
pub struct ContainerInfo {
    /// Short (12-char) container ID
    pub id: String,
    pub name: String,
    pub image: String,
    /// Docker state, e.g. "running", "exited"
    pub state: String,
    /// Human-readable status, e.g. "Up 2 hours"
    pub status: String,
}

impl From<ContainerSummary> for ContainerInfo {
    fn from(c: ContainerSummary) -> Self {
        Self {
            id: c.id.unwrap_or_default().chars().take(12).collect(),
            name: c
                .names
                .unwrap_or_default()
                .first()
                .map(|n| n.trim_start_matches('/').to_string())
                .unwrap_or_else(|| "unnamed".to_string()),
            image: c.image.unwrap_or_default(),
            state: c.state.map(|s| s.to_string()).unwrap_or_default(),
            status: c.status.unwrap_or_default(),
        }
    }
}

/// Lists all containers (running and stopped) on the local Docker daemon.
pub async fn list_containers() -> Result<Vec<ContainerInfo>, String> {
    let docker = Docker::connect_with_local_defaults().map_err(|e| {
        log::error!("Failed to connect to Docker: {e}");
        format!("Failed to connect to Docker: {e}")
    })?;

    let options = ListContainersOptions {
        all: true,
        ..Default::default()
    };

    let containers = docker.list_containers(Some(options)).await.map_err(|e| {
        log::error!("Failed to list containers: {e}");
        format!("Failed to list containers: {e}")
    })?;

    log::debug!("Listed {} containers", containers.len());
    Ok(containers.into_iter().map(ContainerInfo::from).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bollard::models::ContainerSummaryStateEnum;

    #[test]
    fn converts_container_summary() {
        let summary = ContainerSummary {
            id: Some("0123456789abcdef0123".to_string()),
            names: Some(vec!["/research-agent".to_string()]),
            image: Some("nandi/research:latest".to_string()),
            state: Some(ContainerSummaryStateEnum::RUNNING),
            status: Some("Up 2 hours".to_string()),
            ..Default::default()
        };

        let info = ContainerInfo::from(summary);

        assert_eq!(info.id, "0123456789ab");
        assert_eq!(info.name, "research-agent");
        assert_eq!(info.image, "nandi/research:latest");
        assert_eq!(info.state, "running");
        assert_eq!(info.status, "Up 2 hours");
    }

    #[test]
    fn defaults_missing_fields() {
        let info = ContainerInfo::from(ContainerSummary::default());

        assert_eq!(info.id, "");
        assert_eq!(info.name, "unnamed");
        assert_eq!(info.state, "");
    }
}
