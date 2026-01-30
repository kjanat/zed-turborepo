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

/// Turbo icon SVG embedded at compile time
const ICON_SVG: &str = include_str!("../../../resources/icon.svg");

/// Resource URIs
const URI_CONFIG: &str = "turbo://config";
const URI_TASKS: &str = "turbo://tasks";
const URI_PACKAGES: &str = "turbo://packages";

// ============================================================================
// Server State
// ============================================================================

#[derive(Clone)]
pub struct TurboServer {
    cwd: Arc<Mutex<PathBuf>>,
    tool_router: ToolRouter<Self>,
}

impl Default for TurboServer {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Tool Parameters
// ============================================================================

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

// ============================================================================
// Server Implementation
// ============================================================================

#[tool_router]
impl TurboServer {
    #[must_use]
    pub fn new() -> Self {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        Self {
            cwd: Arc::new(Mutex::new(cwd)),
            tool_router: Self::tool_router(),
        }
    }

    async fn find_turbo_json(&self) -> Option<PathBuf> {
        let mut current = self.cwd.lock().await.clone();
        loop {
            for name in ["turbo.json", "turbo.jsonc"] {
                let path = current.join(name);
                if path.exists() {
                    return Some(path);
                }
            }
            if !current.pop() {
                break;
            }
        }
        None
    }

    async fn read_turbo_json(&self) -> Result<serde_json::Value, McpError> {
        let path = self
            .find_turbo_json()
            .await
            .ok_or_else(|| McpError::resource_not_found("No turbo.json found", None))?;

        let content = tokio::fs::read_to_string(&path)
            .await
            .map_err(|e| McpError::internal_error(format!("Read error: {e}"), None))?;

        let content = strip_json_comments(&content);
        serde_json::from_str(&content)
            .map_err(|e| McpError::internal_error(format!("Parse error: {e}"), None))
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

    // ========================================================================
    // Tools
    // ========================================================================

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
}

// ============================================================================
// ServerHandler
// ============================================================================

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
                name: "turbo-mcp".into(),
                version: env!("CARGO_PKG_VERSION").into(),
                title: Some("Turbo MCP Server".into()),
                icons: Some(vec![Icon {
                    src: format!("data:image/svg+xml;base64,{}", BASE64.encode(ICON_SVG)),
                    mime_type: Some("image/svg+xml".into()),
                    sizes: Some(vec!["any".into()]),
                }]),
                website_url: Some("https://github.com/kjanat/zed-turborepo".into()),
            },
            instructions: Some(
                "Turbo MCP server. Resources: turbo://config, turbo://tasks, turbo://packages. \
                 Tools: workdir, daemon, run, graph."
                    .into(),
            ),
        }
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _ctx: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, McpError> {
        Ok(ListResourcesResult {
            resources: vec![
                Annotated::new(
                    RawResource {
                        uri: URI_CONFIG.into(),
                        name: "Turbo Config".into(),
                        description: Some("Full turbo.json".into()),
                        mime_type: Some("application/json".into()),
                        size: None,
                        title: None,
                        icons: None,
                        meta: None,
                    },
                    None,
                ),
                Annotated::new(
                    RawResource {
                        uri: URI_TASKS.into(),
                        name: "Task List".into(),
                        description: Some("Defined tasks".into()),
                        mime_type: Some("application/json".into()),
                        size: None,
                        title: None,
                        icons: None,
                        meta: None,
                    },
                    None,
                ),
                Annotated::new(
                    RawResource {
                        uri: URI_PACKAGES.into(),
                        name: "Packages".into(),
                        description: Some("Workspace packages".into()),
                        mime_type: Some("application/json".into()),
                        size: None,
                        title: None,
                        icons: None,
                        meta: None,
                    },
                    None,
                ),
            ],
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
            URI_CONFIG => {
                let config = self.read_turbo_json().await?;
                Ok(ReadResourceResult {
                    contents: vec![ResourceContents::text(
                        serde_json::to_string_pretty(&config).unwrap(),
                        request.uri,
                    )],
                })
            }
            URI_TASKS => {
                let config = self.read_turbo_json().await?;
                let tasks: Vec<&str> = config
                    .get("tasks")
                    .or_else(|| config.get("pipeline"))
                    .and_then(serde_json::Value::as_object)
                    .map(|t| t.keys().map(String::as_str).collect())
                    .unwrap_or_default();

                Ok(ReadResourceResult {
                    contents: vec![ResourceContents::text(
                        serde_json::to_string_pretty(&tasks).unwrap(),
                        request.uri,
                    )],
                })
            }
            URI_PACKAGES => {
                let cwd = self.cwd.lock().await.clone();
                let output = tokio::process::Command::new("turbo")
                    .args(["ls", "--output", "json"])
                    .current_dir(&cwd)
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .output()
                    .await;

                let content = match output {
                    Ok(out) if out.status.success() => {
                        String::from_utf8_lossy(&out.stdout).to_string()
                    }
                    _ => {
                        // Fallback: package.json workspaces
                        let pkg_path = cwd.join("package.json");
                        tokio::fs::read_to_string(&pkg_path)
                            .await
                            .ok()
                            .and_then(|c| serde_json::from_str::<serde_json::Value>(&c).ok())
                            .and_then(|v| v.get("workspaces").cloned())
                            .map_or_else(
                                || "[]".into(),
                                |w| serde_json::to_string_pretty(&w).unwrap(),
                            )
                    }
                };

                Ok(ReadResourceResult {
                    contents: vec![ResourceContents::text(content, request.uri)],
                })
            }
            _ => Err(McpError::resource_not_found(
                format!("Unknown: {}", request.uri),
                None,
            )),
        }
    }
}

// ============================================================================
// Utilities
// ============================================================================

fn strip_json_comments(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    let mut in_string = false;
    let mut escape_next = false;

    while let Some(c) = chars.next() {
        if escape_next {
            result.push(c);
            escape_next = false;
            continue;
        }
        if c == '\\' && in_string {
            result.push(c);
            escape_next = true;
            continue;
        }
        if c == '"' {
            in_string = !in_string;
            result.push(c);
            continue;
        }
        if in_string {
            result.push(c);
            continue;
        }
        if c == '/' {
            match chars.peek() {
                Some('/') => {
                    chars.next();
                    for next in chars.by_ref() {
                        if next == '\n' {
                            break;
                        }
                    }
                }
                Some('*') => {
                    chars.next();
                    while let Some(next) = chars.next() {
                        if next == '*' && chars.peek() == Some(&'/') {
                            chars.next();
                            break;
                        }
                    }
                }
                _ => result.push(c),
            }
        } else {
            result.push(c);
        }
    }
    result
}

// ============================================================================
// Main
// ============================================================================

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .with_writer(std::io::stderr)
        .init();

    tracing::info!("Starting turbo-mcp v{}", env!("CARGO_PKG_VERSION"));

    let service = TurboServer::new().serve(stdio()).await.inspect_err(|e| {
        tracing::error!("Server error: {e}");
    })?;

    service.waiting().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
