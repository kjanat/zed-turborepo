//! Turbo MCP Server
//!
//! An MCP (Model Context Protocol) server for Turborepo monorepos.
//!
//! ## Resources (read-only data)
//! - `turbo://config` - Full turbo.json configuration
//! - `turbo://tasks` - List of defined tasks
//! - `turbo://packages` - Workspace packages
//!
//! ## Tools (actions)
//! - `workdir` - Manage working directory (get/set)
//! - `daemon` - Control turbo daemon (status/start/stop)
//! - `run` - Execute turbo tasks
//! - `graph` - Show task dependency graph

use std::{path::PathBuf, process::Stdio, sync::Arc};

use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use rmcp::{
    ErrorData as McpError, ServiceExt,
    handler::server::{tool::ToolRouter, wrapper::Parameters},
    model::{
        Annotated, CallToolResult, Content, Icon, Implementation, ListResourcesResult,
        PaginatedRequestParams, ProtocolVersion, RawResource, ReadResourceRequestParams,
        ReadResourceResult, ResourceContents, ServerCapabilities, ServerInfo,
    },
    schemars::JsonSchema,
    service::{RequestContext, RoleServer},
    tool, tool_handler, tool_router,
    transport::stdio,
};
use serde::Deserialize;
use tokio::sync::Mutex;
use turbo_core::{PackageDiscovery, TurboConfig};

/// Turbo icon SVG embedded at compile time
const ICON_SVG: &str = include_str!("../../../resources/icon.svg");

/// Resource metadata definition
struct ResourceDef {
    uri: &'static str,
    name: &'static str,
    description: &'static str,
}

/// All available resources - single source of truth for `list_resources` and instructions
const RESOURCE_DEFS: &[ResourceDef] = &[
    ResourceDef {
        uri: "turbo://config",
        name: "Turbo Config",
        description: "Full turbo.json",
    },
    ResourceDef {
        uri: "turbo://tasks",
        name: "Task List",
        description: "Defined tasks",
    },
    ResourceDef {
        uri: "turbo://packages",
        name: "Packages",
        description: "Workspace packages",
    },
    ResourceDef {
        uri: "turbo://cache",
        name: "Cache Status",
        description: "Cache configuration and status",
    },
];

#[derive(Clone)]
pub struct TurboServer {
    cwd: Arc<Mutex<PathBuf>>,
    tool_router: ToolRouter<Self>,
    instructions: String,
}

impl Default for TurboServer {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WorkdirAction {
    /// Get current working directory
    Get,
    /// Set working directory
    Set,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct WorkdirParams {
    /// Action: "get" or "set"
    pub action: WorkdirAction,
    /// Path (required for set)
    #[serde(default)]
    pub path: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DaemonAction {
    /// Check daemon status
    Status,
    /// Start daemon
    Start,
    /// Stop daemon
    Stop,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DaemonParams {
    /// Action: "status", "start", or "stop"
    pub action: DaemonAction,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RunParams {
    /// Tasks to run (e.g., `["build", "test"]`)
    pub tasks: Vec<String>,
    /// Filter packages (e.g., `@myapp/*`)
    #[serde(default)]
    pub filter: Option<String>,
    /// Dry-run mode
    #[serde(default)]
    pub dry_run: bool,
    /// Continue on error
    #[serde(default)]
    pub continue_on_error: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GraphParams {
    /// Task to graph (default: build)
    #[serde(default)]
    pub task: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PruneParams {
    /// Package scope to prune to
    pub scope: String,
    /// Output directory (default: out)
    #[serde(default)]
    pub out_dir: Option<String>,
    /// Include docker output
    #[serde(default)]
    pub docker: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct QueryParams {
    /// Query string (e.g., "Why does A depend on B?")
    pub query: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct LintParams {
    /// Specific packages to lint (empty = all)
    #[serde(default)]
    pub packages: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct InfoParams {
    /// Package name (empty = workspace info)
    #[serde(default)]
    pub package: Option<String>,
}

#[tool_router]
impl TurboServer {
    #[must_use]
    pub fn new() -> Self {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let tool_router = Self::tool_router();

        // Build instructions dynamically from registered tools and resources
        let resource_uris: Vec<_> = RESOURCE_DEFS.iter().map(|r| r.uri).collect();
        let tools = tool_router.list_all();
        let tool_names: Vec<_> = tools.iter().map(|t| t.name.as_ref()).collect();

        let instructions = format!(
            "{}. Resources: {}. Tools: {}.",
            env!("CARGO_PKG_DESCRIPTION"),
            resource_uris.join(", "),
            tool_names.join(", ")
        );

        Self {
            cwd: Arc::new(Mutex::new(cwd)),
            tool_router,
            instructions,
        }
    }

    /// Load turbo config using turbo-core
    async fn load_config(&self) -> Result<TurboConfig, McpError> {
        let cwd = self.cwd.lock().await.clone();
        TurboConfig::find_and_load(&cwd)
            .await
            .map_err(|e| McpError::resource_not_found(e.to_string(), None))
    }

    /// Get package discovery instance
    async fn discovery(&self) -> PackageDiscovery {
        let cwd = self.cwd.lock().await.clone();
        PackageDiscovery::new(cwd)
    }

    async fn run_turbo(&self, args: &[&str]) -> Result<std::process::Output, McpError> {
        let cwd = self.cwd.lock().await.clone();
        tokio::process::Command::new("turbo")
            .args(args)
            .current_dir(&cwd)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| McpError::internal_error(format!("turbo error: {e}"), None))
    }

    #[tool(description = "Manage working directory (get/set)")]
    async fn workdir(
        &self,
        Parameters(p): Parameters<WorkdirParams>,
    ) -> Result<CallToolResult, McpError> {
        match p.action {
            WorkdirAction::Get => {
                let cwd = self.cwd.lock().await;
                Ok(CallToolResult::success(vec![Content::text(
                    cwd.display().to_string(),
                )]))
            }
            WorkdirAction::Set => {
                let path_str = p
                    .path
                    .ok_or_else(|| McpError::invalid_params("path required for set", None))?;

                let path = PathBuf::from(&path_str);
                let path = if path.is_absolute() {
                    path
                } else {
                    self.cwd.lock().await.join(&path)
                };

                if !path.is_dir() {
                    return Err(McpError::invalid_params(
                        format!("Not a directory: {}", path.display()),
                        None,
                    ));
                }

                let canonical = path
                    .canonicalize()
                    .map_err(|e| McpError::internal_error(format!("Path error: {e}"), None))?;

                *self.cwd.lock().await = canonical.clone();
                Ok(CallToolResult::success(vec![Content::text(format!(
                    "Set to: {}",
                    canonical.display()
                ))]))
            }
        }
    }

    #[tool(description = "Control turbo daemon (status/start/stop)")]
    async fn daemon(
        &self,
        Parameters(p): Parameters<DaemonParams>,
    ) -> Result<CallToolResult, McpError> {
        let args: &[&str] = match p.action {
            DaemonAction::Status => &["daemon", "status"],
            DaemonAction::Start => &["daemon", "start"],
            DaemonAction::Stop => &["daemon", "stop"],
        };

        let output = self.run_turbo(args).await?;
        let response = serde_json::json!({
            "success": output.status.success(),
            "stdout": String::from_utf8_lossy(&output.stdout).trim(),
            "stderr": String::from_utf8_lossy(&output.stderr).trim()
        });

        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&response).unwrap(),
        )]))
    }

    #[tool(description = "Execute turbo tasks")]
    async fn run(&self, Parameters(p): Parameters<RunParams>) -> Result<CallToolResult, McpError> {
        if p.tasks.is_empty() {
            return Err(McpError::invalid_params("No tasks specified", None));
        }

        let cwd = self.cwd.lock().await.clone();
        let mut cmd = tokio::process::Command::new("turbo");
        cmd.arg("run").args(&p.tasks);

        if let Some(filter) = &p.filter {
            cmd.arg("--filter").arg(filter);
        }
        if p.dry_run {
            cmd.arg("--dry-run");
        }
        if p.continue_on_error {
            cmd.arg("--continue");
        }

        let output = cmd
            .current_dir(&cwd)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| McpError::internal_error(format!("Exec error: {e}"), None))?;

        let response = serde_json::json!({
            "tasks": p.tasks,
            "success": output.status.success(),
            "exit_code": output.status.code(),
            "stdout": String::from_utf8_lossy(&output.stdout),
            "stderr": String::from_utf8_lossy(&output.stderr)
        });

        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&response).unwrap(),
        )]))
    }

    #[tool(description = "Show task dependency graph")]
    async fn graph(
        &self,
        Parameters(p): Parameters<GraphParams>,
    ) -> Result<CallToolResult, McpError> {
        let task = p.task.as_deref().unwrap_or("build");
        let output = self
            .run_turbo(&["run", task, "--dry-run", "--graph"])
            .await?;
        Ok(CallToolResult::success(vec![Content::text(
            String::from_utf8_lossy(&output.stdout).to_string(),
        )]))
    }

    #[tool(description = "Prune workspace to minimal subset for a package")]
    async fn prune(
        &self,
        Parameters(p): Parameters<PruneParams>,
    ) -> Result<CallToolResult, McpError> {
        let cwd = self.cwd.lock().await.clone();
        let mut cmd = tokio::process::Command::new("turbo");
        cmd.arg("prune").arg(&p.scope);

        if let Some(out) = &p.out_dir {
            cmd.arg("--out-dir").arg(out);
        }
        if p.docker {
            cmd.arg("--docker");
        }

        let output = cmd
            .current_dir(&cwd)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| McpError::internal_error(format!("Exec error: {e}"), None))?;

        let response = serde_json::json!({
            "scope": p.scope,
            "success": output.status.success(),
            "stdout": String::from_utf8_lossy(&output.stdout),
            "stderr": String::from_utf8_lossy(&output.stderr)
        });

        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&response).unwrap(),
        )]))
    }

    #[tool(description = "Query the task graph (e.g., 'Why does A depend on B?')")]
    async fn query(
        &self,
        Parameters(p): Parameters<QueryParams>,
    ) -> Result<CallToolResult, McpError> {
        let output = self.run_turbo(&["query", &p.query]).await?;

        let response = serde_json::json!({
            "query": p.query,
            "success": output.status.success(),
            "result": String::from_utf8_lossy(&output.stdout),
            "stderr": String::from_utf8_lossy(&output.stderr)
        });

        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&response).unwrap(),
        )]))
    }

    #[tool(description = "Run turbo lint to check configuration")]
    async fn lint(
        &self,
        Parameters(p): Parameters<LintParams>,
    ) -> Result<CallToolResult, McpError> {
        let cwd = self.cwd.lock().await.clone();
        let mut cmd = tokio::process::Command::new("turbo");
        cmd.arg("lint");

        for pkg in &p.packages {
            cmd.arg("--filter").arg(pkg);
        }

        let output = cmd
            .current_dir(&cwd)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| McpError::internal_error(format!("Exec error: {e}"), None))?;

        let response = serde_json::json!({
            "success": output.status.success(),
            "stdout": String::from_utf8_lossy(&output.stdout),
            "stderr": String::from_utf8_lossy(&output.stderr)
        });

        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&response).unwrap(),
        )]))
    }

    #[tool(description = "Get package or workspace info")]
    async fn info(
        &self,
        Parameters(p): Parameters<InfoParams>,
    ) -> Result<CallToolResult, McpError> {
        let discovery = self.discovery().await;

        // Get packages using turbo-core discovery
        let packages = discovery
            .discover_packages()
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        let pkg_info = p.package.as_ref().map_or_else(
            || packages.first(),
            |name| packages.iter().find(|pkg| &pkg.name == name),
        );

        // Get turbo config
        let turbo_config = self.load_config().await.ok();

        let response = serde_json::json!({
            "package": pkg_info,
            "turbo_config": turbo_config
        });

        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&response).unwrap(),
        )]))
    }
}

#[tool_handler]
impl rmcp::ServerHandler for TurboServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: ProtocolVersion::LATEST,
            capabilities: ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
            server_info: Implementation {
                name: env!("CARGO_PKG_NAME").into(),
                version: env!("CARGO_PKG_VERSION").into(),
                title: Some(env!("CARGO_PKG_DESCRIPTION").into()),
                icons: Some(vec![Icon {
                    src: format!("data:image/svg+xml;base64,{}", BASE64.encode(ICON_SVG)),
                    mime_type: Some("image/svg+xml".into()),
                    sizes: Some(vec!["any".into()]),
                }]),
                website_url: Some(env!("CARGO_PKG_REPOSITORY").into()),
            },
            instructions: Some(self.instructions.clone()),
        }
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _ctx: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, McpError> {
        let resources = RESOURCE_DEFS
            .iter()
            .map(|def| {
                Annotated::new(
                    RawResource {
                        uri: def.uri.into(),
                        name: def.name.into(),
                        description: Some(def.description.into()),
                        mime_type: Some("application/json".into()),
                        size: None,
                        title: None,
                        icons: None,
                        meta: None,
                    },
                    None,
                )
            })
            .collect();

        Ok(ListResourcesResult {
            resources,
            next_cursor: None,
            meta: None,
        })
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _ctx: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResult, McpError> {
        match request.uri.as_str() {
            "turbo://config" => {
                let config = self.load_config().await?;
                Ok(ReadResourceResult {
                    contents: vec![ResourceContents::text(
                        serde_json::to_string_pretty(&config).unwrap(),
                        request.uri,
                    )],
                })
            }
            "turbo://tasks" => {
                let config = self.load_config().await?;
                let tasks = config.task_names();

                Ok(ReadResourceResult {
                    contents: vec![ResourceContents::text(
                        serde_json::to_string_pretty(&tasks).unwrap(),
                        request.uri,
                    )],
                })
            }
            "turbo://packages" => {
                let discovery = self.discovery().await;
                let packages = discovery
                    .discover_packages()
                    .await
                    .map_err(|e| McpError::internal_error(e.to_string(), None))?;

                Ok(ReadResourceResult {
                    contents: vec![ResourceContents::text(
                        serde_json::to_string_pretty(&packages).unwrap(),
                        request.uri,
                    )],
                })
            }
            "turbo://cache" => {
                let config = self.load_config().await.ok();
                let cwd = self.cwd.lock().await.clone();

                // Check daemon status for cache info
                let daemon_output = tokio::process::Command::new("turbo")
                    .args(["daemon", "status"])
                    .current_dir(&cwd)
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .output()
                    .await
                    .ok();

                let daemon_status = daemon_output
                    .as_ref()
                    .map(|o| String::from_utf8_lossy(&o.stdout).to_string());

                let response = serde_json::json!({
                    "cacheDir": config.as_ref().map(turbo_core::TurboConfig::cache_dir),
                    "remoteCache": config.as_ref().and_then(|c| c.remote_cache.as_ref()),
                    "daemonStatus": daemon_status
                });

                Ok(ReadResourceResult {
                    contents: vec![ResourceContents::text(
                        serde_json::to_string_pretty(&response).unwrap(),
                        request.uri,
                    )],
                })
            }
            _ => Err(McpError::resource_not_found(
                format!("Unknown: {}", request.uri),
                None,
            )),
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .with_writer(std::io::stderr)
        .init();

    tracing::info!(
        "Starting {} v{}",
        env!("CARGO_PKG_NAME"),
        env!("CARGO_PKG_VERSION")
    );

    let service = TurboServer::new().serve(stdio()).await.inspect_err(|e| {
        tracing::error!("Server error: {e}");
    })?;

    service.waiting().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use turbo_core::config::strip_json_comments;

    #[test]
    fn test_strip_json_comments() {
        let input = r#"{"key": "value" /* comment */}"#;
        assert!(!strip_json_comments(input).contains("/*"));
    }

    #[test]
    fn test_strip_preserves_urls() {
        let input = r#"{"url": "https://example.com"}"#;
        assert_eq!(strip_json_comments(input), input);
    }
}
