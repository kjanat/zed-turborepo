use std::fs;

use schemars::JsonSchema;
use serde::Deserialize;
use zed_extension_api::{
    self as zed, ContextServerConfiguration, ContextServerId, LanguageServerId,
    LanguageServerInstallationStatus, Project, Result,
    process::Command,
    serde_json::{self},
    settings::{ContextServerSettings, LspSettings},
};

/// Settings for the turbo-mcp context server
#[derive(Debug, Deserialize, JsonSchema)]
#[allow(dead_code)] // Only used for schema generation
struct TurboMcpSettings {
    /// Path to the turbo-mcp binary
    binary_path: Option<String>,
}

/// Default settings JSONC for the MCP server configuration UI
const DEFAULT_MCP_SETTINGS: &str = r#"{
  /// Path to the turbo-mcp binary (optional)
  /// If not set, the extension will look for turbo-mcp in your PATH
  // "binary_path": "/path/to/turbo-mcp"
}"#;

const LSP_SERVER_ID: &str = "turbo-lsp";
const MCP_SERVER_ID: &str = "turbo-mcp";
const MCP_BINARY_NAME: &str = "turbo-mcp";

const LSP_SETTINGS_EXAMPLE: &str = r#"   {
      "lsp": {
        "turbo-lsp": {
          "binary": { "path": "/path/to/turbo-lsp" }
        }
      }
    }"#;

const LSP_BINARY_NAMES: &[&str] = &[
    "turbo-lsp",
    "turborepo-lsp",
    "turborepo-lsp-linux-x64",
    "turborepo-lsp-linux-arm64",
    "turborepo-lsp-darwin-x64",
    "turborepo-lsp-darwin-arm64",
    "turborepo-lsp-win32-x64.exe",
];

struct TurboExtension {
    cached_lsp_binary_path: Option<String>,
    cached_mcp_binary_path: Option<String>,
}

impl TurboExtension {
    fn language_server_binary_path(
        &mut self,
        language_server_id: &LanguageServerId,
        worktree: &zed::Worktree,
    ) -> Result<String> {
        if let Ok(lsp_settings) = LspSettings::for_worktree(LSP_SERVER_ID, worktree)
            && let Some(binary) = lsp_settings.binary
            && let Some(path) = binary.path
            && fs::metadata(&path).is_ok_and(|metadata| metadata.is_file())
        {
            return Ok(path);
        }

        for binary_name in LSP_BINARY_NAMES {
            if let Some(path) = worktree.which(binary_name) {
                self.cached_lsp_binary_path = Some(path.clone());
                return Ok(path);
            }
        }

        if let Some(path) = &self.cached_lsp_binary_path
            && fs::metadata(path).is_ok_and(|metadata| metadata.is_file())
        {
            return Ok(path.clone());
        }

        let error = format!(
            "No local turbo LSP binary found. Build `turbo-lsp`, install it on PATH, or set a local path in settings.json.\n\nExample:\n```jsonc\n{LSP_SETTINGS_EXAMPLE}\n```\n\nUpstream binary names also work locally: `turborepo-lsp`, `turborepo-lsp-{}`.",
            Self::platform_binary_name_hint()?
        );

        zed::set_language_server_installation_status(
            language_server_id,
            &LanguageServerInstallationStatus::Failed(error.clone()),
        );

        Err(error)
    }

    fn platform_binary_name_hint() -> Result<&'static str> {
        let (platform, arch) = zed::current_platform();

        match (platform, arch) {
            (zed::Os::Linux, zed::Architecture::X8664) => Ok("linux-x64"),
            (zed::Os::Linux, zed::Architecture::Aarch64) => Ok("linux-arm64"),
            (zed::Os::Mac, zed::Architecture::X8664) => Ok("darwin-x64"),
            (zed::Os::Mac, zed::Architecture::Aarch64) => Ok("darwin-arm64"),
            (zed::Os::Windows, zed::Architecture::X8664) => Ok("win32-x64.exe"),
            (zed::Os::Windows, zed::Architecture::Aarch64) => Ok("win32-arm64.exe"),
            (_, zed::Architecture::X86) => {
                Err("32-bit x86 is not supported by turborepo-lsp".into())
            }
        }
    }

    /// Find the turbo-mcp binary for the MCP server
    ///
    /// Search order:
    /// 1. Custom path from context server settings
    /// 2. Cached path from previous lookup
    /// 3. Default to "turbo-mcp" (resolved from PATH by OS)
    fn mcp_server_binary_path(&self, project: &Project) -> String {
        if let Ok(settings) = ContextServerSettings::for_project(MCP_SERVER_ID, project)
            && let Some(settings) = settings.settings
            && let Some(path) = settings.get("binary_path").and_then(|value| value.as_str())
            && fs::metadata(path).is_ok_and(|metadata| metadata.is_file())
        {
            return path.to_string();
        }

        if let Some(path) = &self.cached_mcp_binary_path
            && fs::metadata(path).is_ok_and(|metadata| metadata.is_file())
        {
            return path.clone();
        }

        MCP_BINARY_NAME.to_string()
    }

    /// Ensure the turbo daemon is running (required by turborepo-lsp)
    fn ensure_daemon_running(worktree: &zed::Worktree) {
        if let Some(turbo_path) = worktree.which("turbo") {
            let _ = Command::new(&turbo_path).args(["daemon", "start"]).output();
        }
    }
}

impl zed::Extension for TurboExtension {
    fn new() -> Self {
        Self {
            cached_lsp_binary_path: None,
            cached_mcp_binary_path: None,
        }
    }

    fn language_server_command(
        &mut self,
        language_server_id: &LanguageServerId,
        worktree: &zed::Worktree,
    ) -> Result<zed::Command> {
        Self::ensure_daemon_running(worktree);

        let binary_path = self.language_server_binary_path(language_server_id, worktree)?;
        let args = if let Ok(lsp_settings) = LspSettings::for_worktree(LSP_SERVER_ID, worktree) {
            lsp_settings
                .binary
                .and_then(|binary| binary.arguments)
                .unwrap_or_default()
        } else {
            Vec::new()
        };

        Ok(zed::Command {
            command: binary_path,
            args,
            env: Vec::new(),
        })
    }

    fn label_for_completion(
        &self,
        _language_server_id: &LanguageServerId,
        completion: zed::lsp::Completion,
    ) -> Option<zed::CodeLabel> {
        let label = &completion.label;

        if let Some((package, task)) = label.split_once('#') {
            Some(zed::CodeLabel {
                code: label.clone(),
                spans: vec![
                    zed::CodeLabelSpan::literal(package, Some("property".to_string())),
                    zed::CodeLabelSpan::literal("#", Some("punctuation".to_string())),
                    zed::CodeLabelSpan::literal(task, Some("function".to_string())),
                ],
                filter_range: (0..label.len()).into(),
            })
        } else {
            Some(zed::CodeLabel {
                code: label.clone(),
                spans: vec![zed::CodeLabelSpan::literal(
                    label,
                    Some("function".to_string()),
                )],
                filter_range: (0..label.len()).into(),
            })
        }
    }

    fn context_server_command(
        &mut self,
        _context_server_id: &ContextServerId,
        project: &Project,
    ) -> Result<zed::Command> {
        Ok(zed::Command {
            command: self.mcp_server_binary_path(project),
            args: vec![],
            env: vec![],
        })
    }

    fn context_server_configuration(
        &mut self,
        _context_server_id: &ContextServerId,
        _project: &Project,
    ) -> Result<Option<ContextServerConfiguration>> {
        Ok(Some(ContextServerConfiguration {
            installation_instructions: include_str!(
                "../configuration/installation_instructions.md"
            )
            .to_string(),
            default_settings: DEFAULT_MCP_SETTINGS.to_string(),
            settings_schema: serde_json::to_string(&schemars::schema_for!(TurboMcpSettings))
                .map_err(|error| error.to_string())?,
        }))
    }
}

zed::register_extension!(TurboExtension);

#[cfg(test)]
mod tests {
    use indexmap::IndexSet;

    use super::*;

    /// Extract keys from JSONC:
    /// - `// "key":` = commented-out key (single //)
    /// - `"key":` = active key
    /// - `/// ...` = doc comment (ignored)
    fn extract_jsonc_keys(input: &str) -> IndexSet<String> {
        let mut keys = IndexSet::new();
        for line in input.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("///") {
                continue;
            }
            if let Some(rest) = trimmed.strip_prefix("//") {
                if let Some(key) = extract_key(rest.trim()) {
                    keys.insert(key);
                }
            } else if let Some(key) = extract_key(trimmed) {
                keys.insert(key);
            }
        }
        keys
    }

    /// Extract key from `"key": ...` pattern
    fn extract_key(input: &str) -> Option<String> {
        let input = input.trim();
        if let Some(stripped) = input.strip_prefix('"')
            && let Some(end) = stripped.find('"')
        {
            let key = &stripped[..end];
            if stripped[end + 1..].trim_start().starts_with(':') {
                return Some(key.to_string());
            }
        }
        None
    }

    #[test]
    fn default_settings_keys_match_schema() {
        let schema = schemars::schema_for!(TurboMcpSettings);
        let schema_json = serde_json::to_value(&schema).unwrap();
        let schema_props = schema_json["properties"]
            .as_object()
            .expect("schema should have properties");
        let schema_keys: IndexSet<_> = schema_props.keys().cloned().collect();

        let settings_keys = extract_jsonc_keys(DEFAULT_MCP_SETTINGS);

        for key in &settings_keys {
            assert!(
                schema_keys.contains(key),
                "default settings key '{key}' not found in schema. Schema keys: {schema_keys:?}"
            );
        }

        for key in &schema_keys {
            assert!(
                settings_keys.contains(key),
                "schema key '{key}' not in default settings. Settings keys: {settings_keys:?}"
            );
        }
    }
}
