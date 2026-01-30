//! Turbo MCP Server
//!
//! An MCP (Model Context Protocol) server that provides tools for interacting
//! with Turborepo monorepos. This server exposes turbo.json configuration and
//! task execution capabilities to AI assistants.

use std::{path::PathBuf, process::Stdio, sync::Arc};

use base64::{Engine, engine::general_purpose::STANDARD as BASE64};

/// Turbo icon SVG embedded at compile time
const ICON_SVG: &str = include_str!("../../../resources/icon.svg");

use rmcp::{
    ErrorData as McpError, ServiceExt,
    handler::server::{tool::ToolRouter, wrapper::Parameters},
    model::{
        CallToolResult, Content, Icon, Implementation, ProtocolVersion, ServerCapabilities,
        ServerInfo,
    },
    schemars::JsonSchema,
    tool, tool_handler, tool_router,
    transport::stdio,
};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

/// The Turbo MCP server state
#[derive(Clone)]
pub struct TurboServer {
    /// Current working directory (monorepo root)
    cwd: Arc<Mutex<PathBuf>>,
    /// Tool router for handling tool calls
    tool_router: ToolRouter<Self>,
}

impl Default for TurboServer {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Tool parameter types
// ============================================================================

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RunTaskParams {
    /// Tasks to run (e.g., `["build", "test", "lint"]`)
    pub tasks: Vec<String>,
    /// Optional filter to run tasks for specific packages
    #[serde(default)]
    pub filter: Option<String>,
    /// Whether to run in dry-run mode (show what would be executed)
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetTaskConfigParams {
    /// The task name to get configuration for
    pub task: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SetWorkdirParams {
    /// The path to set as the working directory
    pub path: String,
}

// ============================================================================
// Response types
// ============================================================================

#[derive(Debug, Serialize)]
pub struct TaskInfo {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub depends_on: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inputs: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outputs: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub persistent: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct PackageInfo {
    pub name: String,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scripts: Option<Vec<String>>,
}

// ============================================================================
// Server implementation
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

    /// Find the turbo.json file in the current directory or parent directories
    async fn find_turbo_json(&self) -> Option<PathBuf> {
        let mut current = self.cwd.lock().await.clone();

        loop {
            let turbo_json = current.join("turbo.json");
            if turbo_json.exists() {
                return Some(turbo_json);
            }

            let turbo_jsonc = current.join("turbo.jsonc");
            if turbo_jsonc.exists() {
                return Some(turbo_jsonc);
            }

            if !current.pop() {
                break;
            }
        }
        None
    }

    /// Read and parse turbo.json
    async fn read_turbo_json(&self) -> Result<serde_json::Value, McpError> {
        let path = self.find_turbo_json().await.ok_or_else(|| {
            McpError::invalid_request("No turbo.json found in current directory or parents", None)
        })?;

        let content = tokio::fs::read_to_string(&path).await.map_err(|e| {
            McpError::internal_error(format!("Failed to read turbo.json: {e}"), None)
        })?;

        // Parse as JSONC (strip comments)
        let content = strip_json_comments(&content);

        serde_json::from_str(&content)
            .map_err(|e| McpError::internal_error(format!("Failed to parse turbo.json: {e}"), None))
    }

    // ========================================================================
    // Tools
    // ========================================================================

    #[tool(description = "List all tasks defined in turbo.json")]
    async fn list_tasks(&self) -> Result<CallToolResult, McpError> {
        let config = self.read_turbo_json().await?;

        let tasks = config
            .get("tasks")
            .or_else(|| config.get("pipeline")) // Legacy support
            .and_then(serde_json::Value::as_object)
            .map(|tasks| tasks.keys().cloned().collect::<Vec<_>>())
            .unwrap_or_default();

        let response = serde_json::json!({
            "tasks": tasks,
            "count": tasks.len()
        });

        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&response).unwrap(),
        )]))
    }

    #[tool(description = "Get detailed configuration for a specific task")]
    async fn get_task_config(
        &self,
        Parameters(params): Parameters<GetTaskConfigParams>,
    ) -> Result<CallToolResult, McpError> {
        let config = self.read_turbo_json().await?;

        let task_config = config
            .get("tasks")
            .or_else(|| config.get("pipeline"))
            .and_then(|t| t.get(&params.task))
            .ok_or_else(|| {
                McpError::invalid_request(format!("Task '{}' not found", params.task), None)
            })?;

        let response = serde_json::json!({
            "task": params.task,
            "config": task_config
        });

        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&response).unwrap(),
        )]))
    }

    #[tool(description = "Get the full turbo.json configuration")]
    async fn get_turbo_config(&self) -> Result<CallToolResult, McpError> {
        let config = self.read_turbo_json().await?;

        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&config).unwrap(),
        )]))
    }

    #[tool(description = "Run turbo tasks (e.g., build, test, lint)")]
    async fn run_task(
        &self,
        Parameters(params): Parameters<RunTaskParams>,
    ) -> Result<CallToolResult, McpError> {
        if params.tasks.is_empty() {
            return Err(McpError::invalid_request("No tasks specified", None));
        }

        let cwd = self.cwd.lock().await.clone();

        let mut cmd = tokio::process::Command::new("turbo");
        cmd.arg("run").args(&params.tasks);

        if let Some(filter) = &params.filter {
            cmd.arg("--filter").arg(filter);
        }

        if params.dry_run {
            cmd.arg("--dry-run");
        }

        cmd.current_dir(&cwd)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let output = cmd
            .output()
            .await
            .map_err(|e| McpError::internal_error(format!("Failed to execute turbo: {e}"), None))?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        let response = serde_json::json!({
            "tasks": params.tasks,
            "success": output.status.success(),
            "exit_code": output.status.code(),
            "stdout": stdout,
            "stderr": stderr
        });

        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&response).unwrap(),
        )]))
    }

    #[tool(description = "List all packages in the monorepo")]
    async fn list_packages(&self) -> Result<CallToolResult, McpError> {
        let cwd = self.cwd.lock().await.clone();

        // Use turbo ls to list packages
        let output = tokio::process::Command::new("turbo")
            .args(["ls", "--output", "json"])
            .current_dir(&cwd)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| {
                McpError::internal_error(format!("Failed to execute turbo ls: {e}"), None)
            })?;

        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            // Try to parse as JSON, otherwise return raw output
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&stdout) {
                return Ok(CallToolResult::success(vec![Content::text(
                    serde_json::to_string_pretty(&json).unwrap(),
                )]));
            }
            return Ok(CallToolResult::success(vec![Content::text(
                stdout.to_string(),
            )]));
        }

        // Fallback: read from package.json workspaces
        let package_json_path = cwd.join("package.json");
        if package_json_path.exists() {
            let content = tokio::fs::read_to_string(&package_json_path)
                .await
                .map_err(|e| {
                    McpError::internal_error(format!("Failed to read package.json: {e}"), None)
                })?;

            let pkg: serde_json::Value = serde_json::from_str(&content).map_err(|e| {
                McpError::internal_error(format!("Failed to parse package.json: {e}"), None)
            })?;

            let workspaces = pkg
                .get("workspaces")
                .map(|w| {
                    w.as_array().map_or_else(
                        || {
                            w.as_object()
                                .and_then(|obj| obj.get("packages"))
                                .and_then(serde_json::Value::as_array)
                                .cloned()
                                .unwrap_or_default()
                        },
                        Clone::clone,
                    )
                })
                .unwrap_or_default();

            let response = serde_json::json!({
                "workspaces": workspaces,
                "source": "package.json"
            });

            return Ok(CallToolResult::success(vec![Content::text(
                serde_json::to_string_pretty(&response).unwrap(),
            )]));
        }

        Err(McpError::internal_error(
            "Could not list packages. Make sure turbo is installed or package.json exists.",
            None,
        ))
    }

    #[tool(description = "Get the current working directory")]
    async fn get_workdir(&self) -> Result<CallToolResult, McpError> {
        let cwd = self.cwd.lock().await;
        Ok(CallToolResult::success(vec![Content::text(
            cwd.display().to_string(),
        )]))
    }

    #[tool(description = "Set the working directory for turbo commands")]
    async fn set_workdir(
        &self,
        Parameters(params): Parameters<SetWorkdirParams>,
    ) -> Result<CallToolResult, McpError> {
        let path = PathBuf::from(&params.path);
        let path = if path.is_absolute() {
            path
        } else {
            let cwd = self.cwd.lock().await;
            cwd.join(&path)
        };

        if !path.exists() {
            return Err(McpError::invalid_request(
                format!("Path does not exist: {}", path.display()),
                None,
            ));
        }

        if !path.is_dir() {
            return Err(McpError::invalid_request(
                format!("Path is not a directory: {}", path.display()),
                None,
            ));
        }

        let canonical = path.canonicalize().map_err(|e| {
            McpError::internal_error(format!("Failed to canonicalize path: {e}"), None)
        })?;

        *self.cwd.lock().await = canonical.clone();

        Ok(CallToolResult::success(vec![Content::text(format!(
            "Working directory set to: {}",
            canonical.display()
        ))]))
    }

    #[tool(description = "Check if turbo daemon is running and get its status")]
    async fn daemon_status(&self) -> Result<CallToolResult, McpError> {
        let cwd = self.cwd.lock().await.clone();

        let output = tokio::process::Command::new("turbo")
            .args(["daemon", "status"])
            .current_dir(&cwd)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| {
                McpError::internal_error(format!("Failed to check daemon status: {e}"), None)
            })?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        let response = serde_json::json!({
            "running": output.status.success(),
            "stdout": stdout.trim(),
            "stderr": stderr.trim()
        });

        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&response).unwrap(),
        )]))
    }

    #[tool(description = "Show the task dependency graph as text")]
    async fn show_graph(&self) -> Result<CallToolResult, McpError> {
        let cwd = self.cwd.lock().await.clone();

        let output = tokio::process::Command::new("turbo")
            .args(["run", "build", "--graph"])
            .current_dir(&cwd)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| {
                McpError::internal_error(format!("Failed to generate graph: {e}"), None)
            })?;

        let stdout = String::from_utf8_lossy(&output.stdout);

        Ok(CallToolResult::success(vec![Content::text(
            stdout.to_string(),
        )]))
    }
}

#[tool_handler]
impl rmcp::ServerHandler for TurboServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: ProtocolVersion::LATEST,
            capabilities: ServerCapabilities::builder().enable_tools().build(),
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
                "Turbo MCP server provides tools for interacting with Turborepo monorepos. \
                 Available tools: list_tasks, get_task_config, get_turbo_config, run_task, \
                 list_packages, get_workdir, set_workdir, daemon_status, show_graph"
                    .into(),
            ),
        }
    }
}

// ============================================================================
// Utilities
// ============================================================================

/// Strip JSON comments (// and /* */) from a string
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
                    // Line comment - skip until newline
                    chars.next();
                    while let Some(&next) = chars.peek() {
                        if next == '\n' {
                            break;
                        }
                        chars.next();
                    }
                }
                Some('*') => {
                    // Block comment - skip until */
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
    // Initialize tracing for logging (to stderr, so it doesn't interfere with MCP)
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .with_writer(std::io::stderr)
        .init();

    tracing::info!("Starting turbo-mcp server v{}", env!("CARGO_PKG_VERSION"));

    // Create and run the server with stdio transport
    let service = TurboServer::new().serve(stdio()).await.inspect_err(|e| {
        tracing::error!("Error starting server: {}", e);
    })?;

    service.waiting().await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_json_comments() {
        let input = r#"{
            // This is a comment
            "key": "value", // inline comment
            /* block
               comment */
            "another": "value"
        }"#;

        let result = strip_json_comments(input);
        assert!(!result.contains("//"));
        assert!(!result.contains("/*"));
        assert!(result.contains(r#""key": "value""#));
        assert!(result.contains(r#""another": "value""#));
    }

    #[test]
    fn test_strip_comments_preserves_strings() {
        let input = r#"{"url": "https://example.com"}"#;
        let result = strip_json_comments(input);
        assert_eq!(input, result);
    }
}
