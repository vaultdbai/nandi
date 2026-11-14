//! External MCP servers (`kind: mcp`): a process over stdio (`config.command`,
//! `args` and `env`) or a Streamable HTTP endpoint (`config.url`). Secrets
//! come from the keyring: for stdio every `secret_refs` key is an environment
//! variable; for HTTP, `secret_refs.bearer_token` is sent as the bearer token.
//!
//! A stdio server runs in its own container ([`Placement::Container`]): the
//! connectors image (`nandi-connectors`: Node, Python and uv, so `npx` and
//! `uvx` work), or the image its `config.image` names when it needs
//! something unusual, read-only, no capabilities, on the internal `nandi-agents` network
//! whose only way out is the egress proxy, as `connector:<id>`. It runs on
//! the host only for agents whose policy lists it in
//! `connectors.run_on_host` (desktop tools).
//!
//! An external server's tools all need approval unless `config.safe_tools`
//! lists them: the server's own hints aren't trusted for that.

use std::collections::HashSet;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::future::BoxFuture;
use rmcp::model::CallToolRequestParams;
use rmcp::service::{RunningService, RxJsonRpcMessage, TxJsonRpcMessage};
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::transport::{StreamableHttpClientTransport, Transport};
use rmcp::{Peer, RoleClient, ServiceExt};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::process::Child;

use super::catalog::{config_list, config_str, ConnectorDefinition};
use super::{Connector, ConnectorDecl, ToolDef};
use crate::sandbox::{SandboxSpec, Tmpfs};
use crate::services::secrets::Secrets;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

/// Newline-delimited JSON-RPC over any reader/writer pair: a child's
/// stdout/stdin, or an in-memory pipe in tests.
pub struct LineTransport<R, W> {
    reader: BufReader<R>,
    writer: Arc<tokio::sync::Mutex<W>>,
}

impl<R, W> LineTransport<R, W>
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    pub fn new(reader: R, writer: W) -> Self {
        Self {
            reader: BufReader::new(reader),
            writer: Arc::new(tokio::sync::Mutex::new(writer)),
        }
    }
}

impl<R, W> Transport<RoleClient> for LineTransport<R, W>
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    type Error = std::io::Error;

    fn send(
        &mut self,
        item: TxJsonRpcMessage<RoleClient>,
    ) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send + 'static {
        let writer = self.writer.clone();
        let line = serde_json::to_string(&item).map(|mut line| {
            line.push('\n');
            line
        });
        async move {
            let line = line.map_err(std::io::Error::other)?;
            let mut writer = writer.lock().await;
            writer.write_all(line.as_bytes()).await?;
            writer.flush().await
        }
    }

    async fn receive(&mut self) -> Option<RxJsonRpcMessage<RoleClient>> {
        loop {
            let mut line = String::new();
            match self.reader.read_line(&mut line).await {
                Ok(0) | Err(_) => return None,
                Ok(_) => {}
            }
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            match serde_json::from_str(line) {
                Ok(message) => return Some(message),
                // Servers sometimes print banners on stdout; skip them
                Err(e) => log::debug!("Skipping a non-JSON-RPC line from an MCP server: {e}"),
            }
        }
    }

    async fn close(&mut self) -> Result<(), Self::Error> {
        self.writer.lock().await.shutdown().await
    }
}

/// Where a stdio MCP server runs
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Placement {
    /// Its own container; `proxy_token` is its egress identity
    /// (`connector:<id>`)
    Container { proxy_token: String },
    /// On the host, as the user (`connectors.run_on_host`)
    Host,
}

/// The egress proxy's name for a connector's container
pub fn proxy_identity(connector_id: &str) -> String {
    format!("{IDENTITY_PREFIX}{connector_id}")
}

/// The connector behind a [`proxy_identity`], if it is one
pub fn connector_of_identity(identity: &str) -> Option<&str> {
    identity.strip_prefix(IDENTITY_PREFIX)
}

const IDENTITY_PREFIX: &str = "connector:";

/// The container a stdio connector runs in
pub fn container_name(connector_id: &str) -> String {
    let safe: String = connector_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();
    format!(
        "nandi-connector-{}-{safe}",
        crate::utils::nandi_config::instance_tag()
    )
}

pub struct McpConnector {
    peer: Peer<RoleClient>,
    service: Mutex<Option<RunningService<RoleClient, ()>>>,
    child: Mutex<Option<Child>>,
    tools: Vec<ToolDef>,
}

impl McpConnector {
    pub async fn connect(
        decl: &ConnectorDecl,
        definition: &ConnectorDefinition,
        secrets: &Secrets,
        placement: &Placement,
    ) -> Result<Self, String> {
        let mut safe: HashSet<String> = config_list(decl, "safe_tools").into_iter().collect();
        for sensitive in &definition.sensitive_tools {
            safe.remove(sensitive);
        }
        if let Some(command) = config_str(decl, "command") {
            let mut env = Vec::new();
            for (name, entry) in &decl.secret_refs {
                let value = secrets.get(entry.as_str())?.ok_or_else(|| {
                    format!(
                        "Connector '{}' needs the keychain entry '{}'",
                        decl.id,
                        entry.as_str()
                    )
                })?;
                env.push((name.clone(), value));
            }
            if let Some(serde_yaml::Value::Mapping(map)) = decl.config.get("env") {
                for (key, value) in map {
                    if let (Some(key), Some(value)) = (key.as_str(), value.as_str()) {
                        env.push((key.to_string(), value.to_string()));
                    }
                }
            }
            let args = config_list(decl, "args");
            return match placement {
                Placement::Host => Self::spawn(&command, &args, env, safe).await,
                Placement::Container { proxy_token } => {
                    let image = config_str(decl, "image")
                        .unwrap_or_else(|| crate::runtime::CONNECTORS_IMAGE.to_string());
                    Self::spawn_in_container(
                        decl.id.as_str(),
                        &image,
                        &command,
                        &args,
                        env,
                        proxy_token,
                        safe,
                    )
                    .await
                }
            };
        }
        if let Some(url) = config_str(decl, "url") {
            let token = match decl.secret_refs.get("bearer_token") {
                Some(entry) => Some(secrets.get(entry.as_str())?.ok_or_else(|| {
                    format!(
                        "Connector '{}' needs the keychain entry '{}'",
                        decl.id,
                        entry.as_str()
                    )
                })?),
                None => None,
            };
            let mut config = StreamableHttpClientTransportConfig::with_uri(url);
            config.auth_header = token;
            let transport = StreamableHttpClientTransport::from_config(config);
            return Self::serve(transport, None, safe).await;
        }
        Err(format!(
            "Connector '{}' needs config.command (stdio) or config.url (HTTP)",
            decl.id
        ))
    }

    async fn spawn(
        command: &str,
        args: &[String],
        env: Vec<(String, String)>,
        safe: HashSet<String>,
    ) -> Result<Self, String> {
        let mut cmd = host_command(command, args);
        cmd.envs(env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("Couldn't start '{command}': {e}"))?;
        let stdin = child.stdin.take().ok_or("No stdin for the MCP server")?;
        let stdout = child.stdout.take().ok_or("No stdout for the MCP server")?;
        if let Some(stderr) = child.stderr.take() {
            let command = command.to_string();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    log::debug!("[{command}] {line}");
                }
            });
        }
        Self::serve(LineTransport::new(stdout, stdin), Some(child), safe).await
    }

    /// Runs the server in its own container (see the module docs). Every
    /// variable is passed as `-e NAME`, so secrets never appear in a
    /// command line.
    async fn spawn_in_container(
        id: &str,
        image: &str,
        command: &str,
        args: &[String],
        mut env: Vec<(String, String)>,
        proxy_token: &str,
        safe: HashSet<String>,
    ) -> Result<Self, String> {
        use crate::services::egress::relay::{self, RelayImage};
        let ours = image == crate::runtime::CONNECTORS_IMAGE;
        // Pulled when missing (progress on `images://pull`)
        crate::services::images::ensure(crate::services::images::SandboxImage::Connectors).await?;
        if !ours && !crate::runtime::image_present(image).await.unwrap_or(false) {
            pull_image(image).await?;
        }
        relay::ensure_ready(RelayImage::Connectors).await?;
        env.extend(
            crate::services::egress::proxy_env(proxy_token)
                .into_iter()
                .filter_map(|pair| {
                    let (name, value) = pair.split_once('=')?;
                    Some((name.to_string(), value.to_string()))
                }),
        );
        let name = container_name(id);
        // A container left from an earlier run holds the name
        let _ = tokio::process::Command::new("docker")
            .args(["rm", "-f", &name])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await;
        let mut cmd = tokio::process::Command::new("docker");
        cmd.args(container_args(id, &name, image, &env, command, args));
        #[cfg(windows)]
        {
            cmd.creation_flags(0x0800_0000);
        }
        cmd.envs(env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = cmd.spawn().map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => "Docker is not installed".to_string(),
            _ => format!("Couldn't start the container for '{command}': {e}"),
        })?;
        let stdin = child.stdin.take().ok_or("No stdin for the MCP server")?;
        let stdout = child.stdout.take().ok_or("No stdout for the MCP server")?;
        if let Some(stderr) = child.stderr.take() {
            let label = format!("{name} {command}");
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    log::debug!("[{label}] {line}");
                }
            });
        }
        Self::serve(LineTransport::new(stdout, stdin), Some(child), safe).await
    }

    /// Initializes the MCP session over `transport` and lists its tools.
    pub async fn serve<T, E, A>(
        transport: T,
        child: Option<Child>,
        safe: HashSet<String>,
    ) -> Result<Self, String>
    where
        T: rmcp::transport::IntoTransport<RoleClient, E, A>,
        E: std::error::Error + Send + Sync + 'static,
    {
        let service = tokio::time::timeout(CONNECT_TIMEOUT, ().serve(transport))
            .await
            .map_err(|_| "The MCP server didn't answer in time".to_string())?
            .map_err(|e| format!("Couldn't connect to the MCP server: {e}"))?;
        let peer = service.peer().clone();
        let tools = peer
            .list_all_tools()
            .await
            .map_err(|e| format!("Couldn't list the MCP server's tools: {e}"))?
            .into_iter()
            .map(|tool| ToolDef {
                sensitive: !safe.contains(tool.name.as_ref()),
                name: tool.name.to_string(),
                description: tool.description.map(|d| d.to_string()).unwrap_or_default(),
                input_schema: Value::Object((*tool.input_schema).clone()),
            })
            .collect();
        Ok(Self {
            peer,
            service: Mutex::new(Some(service)),
            child: Mutex::new(child),
            tools,
        })
    }
}

/// `docker run` for a stdio connector: the agents' hardening, its network,
/// and `env` by name only
fn container_args(
    id: &str,
    name: &str,
    image: &str,
    env: &[(String, String)],
    command: &str,
    args: &[String],
) -> Vec<String> {
    SandboxSpec::new(image)
        .interactive()
        .remove_on_exit()
        .init()
        .name(name)
        .label("nandi.connector", id)
        .tmpfs(Tmpfs::scratch("/tmp", 512))
        // `exec`: Docker mounts a tmpfs noexec by default, and uvx runs the
        // server it installs into ~/.cache
        .tmpfs(Tmpfs::home("/home/node", 512).exec())
        .network(crate::services::egress::relay::AGENT_NETWORK)
        .env_names(env.iter().map(|(name, _)| name))
        // Past any entrypoint the image has
        .entrypoint(command)
        .command(args)
        .to_cli_args()
}

/// `docker pull` for a connector that names its own image
async fn pull_image(image: &str) -> Result<(), String> {
    log::info!("Pulling the connector image {image}");
    let mut cmd = tokio::process::Command::new("docker");
    cmd.args(["pull", "--quiet", image])
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        cmd.creation_flags(0x0800_0000);
    }
    let output = tokio::time::timeout(Duration::from_secs(600), cmd.output())
        .await
        .map_err(|_| format!("Downloading {image} took too long"))?
        .map_err(|e| format!("Couldn't run docker to download {image}: {e}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "Couldn't download the connector's image {image}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

/// On Windows, `npx`/`uvx` are `.cmd` shims that only `cmd` can run, and a
/// console window must not flash up.
pub(crate) fn host_command(command: &str, args: &[String]) -> tokio::process::Command {
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let is_exe = std::path::Path::new(command)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"));
        let mut cmd = if is_exe {
            let mut cmd = tokio::process::Command::new(command);
            cmd.args(args);
            cmd
        } else {
            let mut cmd = tokio::process::Command::new("cmd");
            cmd.arg("/C").arg(command).args(args);
            cmd
        };
        cmd.creation_flags(CREATE_NO_WINDOW);
        cmd
    }
    #[cfg(not(windows))]
    {
        let mut cmd = tokio::process::Command::new(command);
        cmd.args(args);
        cmd
    }
}

/// Structured output if there is any, else the text blocks joined.
fn result_value(result: &Value) -> Result<Value, String> {
    let is_error = result["isError"].as_bool().unwrap_or(false);
    let text = || {
        result["content"]
            .as_array()
            .map(|blocks| {
                blocks
                    .iter()
                    .filter_map(|block| block["text"].as_str())
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default()
    };
    if is_error {
        return Err(text());
    }
    match result.get("structuredContent") {
        Some(structured) if !structured.is_null() => Ok(structured.clone()),
        _ => Ok(Value::String(text())),
    }
}

impl Connector for McpConnector {
    fn list_tools(&self) -> Vec<ToolDef> {
        self.tools.clone()
    }

    fn call_tool<'a>(
        &'a self,
        name: &'a str,
        args: &'a Value,
    ) -> BoxFuture<'a, Result<Value, String>> {
        Box::pin(async move {
            let mut params = CallToolRequestParams::new(name.to_string());
            params.arguments = match args {
                Value::Object(map) => Some(map.clone()),
                Value::Null => None,
                other => {
                    return Err(format!(
                        "Tool arguments must be a JSON object, got: {other}"
                    ))
                }
            };
            let result = self
                .peer
                .call_tool(params)
                .await
                .map_err(|e| format!("Tool '{name}' failed: {e}"))?;
            result_value(&serde_json::to_value(&result).unwrap_or_default())
        })
    }

    fn health_check(&self) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async move {
            if self.peer.is_transport_closed() {
                return Err("The MCP server's connection is closed".to_string());
            }
            self.peer
                .list_tools(None)
                .await
                .map(|_| ())
                .map_err(|e| format!("The MCP server didn't answer: {e}"))
        })
    }

    fn shutdown(&self) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            let service = self
                .service
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .take();
            if let Some(service) = service {
                let _ = service.cancel().await;
            }
            let child = self.child.lock().unwrap_or_else(|e| e.into_inner()).take();
            if let Some(mut child) = child {
                let _ = child.kill().await;
            }
        })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde_json::json;

    /// A minimal MCP server on the other end of an in-memory pipe: an
    /// `echo` tool (safe) and a `delete` tool (fails).
    pub(crate) async fn fake_server() -> McpConnector {
        let (client, server) = tokio::io::duplex(64 * 1024);
        let (server_read, mut server_write) = tokio::io::split(server);
        tokio::spawn(async move {
            let mut lines = BufReader::new(server_read).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let request: Value = serde_json::from_str(&line).unwrap();
                let Some(id) = request.get("id").cloned() else {
                    continue; // notification
                };
                let result = match request["method"].as_str().unwrap() {
                    "initialize" => json!({
                        "protocolVersion": request["params"]["protocolVersion"],
                        "capabilities": {"tools": {}},
                        "serverInfo": {"name": "fake", "version": "1.0.0"},
                    }),
                    "tools/list" => json!({"tools": [
                        {"name": "echo", "description": "Echoes", "inputSchema": {"type": "object"}},
                        {"name": "delete", "inputSchema": {"type": "object"}},
                    ]}),
                    "tools/call" if request["params"]["name"] == "echo" => json!({
                        "content": [{"type": "text", "text": request["params"]["arguments"]["text"]}],
                        "isError": false,
                    }),
                    "tools/call" => json!({
                        "content": [{"type": "text", "text": "nope"}],
                        "isError": true,
                    }),
                    _ => json!({}),
                };
                let response = json!({"jsonrpc": "2.0", "id": id, "result": result});
                let mut out = response.to_string();
                out.push('\n');
                if server_write.write_all(out.as_bytes()).await.is_err() {
                    break;
                }
            }
        });
        let (read, write) = tokio::io::split(client);
        McpConnector::serve(
            LineTransport::new(read, write),
            None,
            HashSet::from(["echo".to_string()]),
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn talks_mcp_over_a_line_transport() {
        let mcp = fake_server().await;
        let tools = mcp.list_tools();
        assert_eq!(tools.len(), 2);
        assert!(!tools[0].sensitive, "echo is in safe_tools");
        assert!(tools[1].sensitive, "everything else needs approval");

        let echoed = mcp.call_tool("echo", &json!({"text": "hi"})).await;
        assert_eq!(echoed.unwrap(), json!("hi"));
        assert_eq!(
            mcp.call_tool("delete", &json!({})).await.unwrap_err(),
            "nope"
        );
        mcp.health_check().await.unwrap();
        mcp.shutdown().await;
    }

    #[test]
    fn connector_containers_are_hardened_and_get_no_secret_values() {
        let env = vec![
            ("GITHUB_TOKEN".to_string(), "ghp_secret".to_string()),
            (
                "HTTPS_PROXY".to_string(),
                "http://nandi:t@nandi-egress:11435".to_string(),
            ),
        ];
        let args = container_args(
            "github",
            "nandi-connector-github",
            crate::runtime::CONNECTORS_IMAGE,
            &env,
            "npx",
            &["-y".to_string(), "@acme/github-mcp".to_string()],
        );
        let joined = args.join(" ");
        assert!(joined.starts_with("run -i --rm --init --name nandi-connector-github"));
        for flag in [
            "--cap-drop=ALL",
            "--security-opt=no-new-privileges",
            "--user=1000:1000",
            "--read-only",
            "--pids-limit=256",
            "--memory=1024m",
            "--network=nandi-agents",
            "--entrypoint=npx",
        ] {
            assert!(joined.contains(flag), "{flag}: {joined}");
        }
        assert!(joined.contains("-e GITHUB_TOKEN -e HTTPS_PROXY"));
        assert!(
            !joined.contains("ghp_secret"),
            "values never in the command line"
        );
        assert!(
            joined.contains("--tmpfs=/home/node:rw,exec,"),
            "uvx runs what it installs in the home folder"
        );
        assert!(joined.ends_with("nandi-connectors:sandbox-6 -y @acme/github-mcp"));
        let tag = crate::utils::nandi_config::instance_tag();
        assert_eq!(
            container_name("a.b c"),
            format!("nandi-connector-{tag}-a-b-c")
        );
    }

    #[test]
    fn connector_identities_round_trip() {
        assert_eq!(
            connector_of_identity(&proxy_identity("github")),
            Some("github")
        );
        assert_eq!(connector_of_identity("writer"), None);
    }

    /// A stdio MCP server in its own container on `nandi-agents`: needs
    /// Docker and `ghcr.io/vaultdbai/nandi-connectors:sandbox-6`.
    /// Run with: cargo test connector_container_e2e -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn connector_container_e2e() {
        const SERVER: &str = "const rl = require('readline').createInterface({ input: process.stdin });\
            rl.on('line', line => { const m = JSON.parse(line); if (m.id === undefined) return;\
            let result = {};\
            if (m.method === 'initialize') result = { protocolVersion: m.params.protocolVersion,\
              capabilities: { tools: {} }, serverInfo: { name: 'e2e', version: '1' } };\
            if (m.method === 'tools/list') result = { tools: [{ name: 'whoami',\
              inputSchema: { type: 'object' } }] };\
            if (m.method === 'tools/call') result = { content: [{ type: 'text',\
              text: String(process.getuid()) + ' ' + (process.env.HTTPS_PROXY || '') }] };\
            process.stdout.write(JSON.stringify({ jsonrpc: '2.0', id: m.id, result }) + '\\n'); });";
        let manifest = json!({"connectors": [{
            "id": "e2e-echo",
            "kind": "mcp",
            "config": {"command": "node", "args": ["-e", SERVER], "safe_tools": ["whoami"]},
        }]});
        let manifest = crate::services::domain::ConnectorManifest::from_yaml(
            &serde_yaml::to_string(&manifest).unwrap(),
        )
        .unwrap();
        let decl = manifest.get("e2e-echo").unwrap();
        let definition = super::super::catalog::definition_for(decl);
        let mcp = McpConnector::connect(
            decl,
            &definition,
            &Secrets::in_memory(),
            &Placement::Container {
                proxy_token: "e2e-token".into(),
            },
        )
        .await
        .unwrap();
        assert_eq!(mcp.list_tools()[0].name, "whoami");
        let said = mcp.call_tool("whoami", &json!({})).await.unwrap();
        let said = said.as_str().unwrap_or_default().to_string();
        assert!(said.starts_with("1000 "), "runs as uid 1000: {said}");
        assert!(
            said.contains("nandi-egress:11435"),
            "through the proxy: {said}"
        );

        let inspect = tokio::process::Command::new("docker")
            .args([
                "inspect",
                "--format",
                "{{.HostConfig.ReadonlyRootfs}} {{.HostConfig.NetworkMode}}",
                &container_name("e2e-echo"),
            ])
            .output()
            .await
            .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&inspect.stdout).trim(),
            "true nandi-agents"
        );
        mcp.shutdown().await;
    }

    #[test]
    fn prefers_structured_content() {
        let value = result_value(&json!({
            "content": [{"type": "text", "text": "{}"}],
            "structuredContent": {"n": 1},
        }));
        assert_eq!(value.unwrap(), json!({"n": 1}));
    }
}
