//! Container commands (`services::containers`).

use crate::services::containers::{self, ContainerInfo};

super::commands! {
    /// Lists all containers (running and stopped) on the local Docker daemon
    fn list_containers() -> Vec<ContainerInfo> => containers::list_containers().await;
}
