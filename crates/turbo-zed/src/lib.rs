use std::fs;

use schemars::JsonSchema;
use serde::Deserialize;
use zed_extension_api::{
    self as zed, ContextServerConfiguration, ContextServerId, DownloadedFileType, LanguageServerId,
    LanguageServerInstallationStatus, Project, Result,
    http_client::{HttpMethod, HttpRequest},
    process::Command,
    serde_json::{self, Value},
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

const MCP_SERVER_ID: &str = "turbo-mcp";
const MCP_BINARY_NAME: &str = "turbo-mcp";

const MARKETPLACE_API_URL: &str =
    "https://marketplace.visualstudio.com/_apis/public/gallery/extensionquery";
const EXTENSION_ID: &str = "vercel.turbo-vsc";

const LSP_SETTINGS_EXAMPLE: &str = r#"   {
     "lsp": {
       "turborepo-lsp": {
         "binary": { "path": "/path/to/turborepo-lsp" }
       }
     }
   }"#;

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
        // Check settings for custom path first
        if let Ok(lsp_settings) = LspSettings::for_worktree("turborepo-lsp", worktree)
            && let Some(binary) = lsp_settings.binary
            && let Some(path) = binary.path
            && fs::metadata(&path).is_ok_and(|m| m.is_file())
        {
            return Ok(path);
        }

        // Check for turborepo-lsp in PATH
        let binary_names = Self::get_binary_names();
        for name in &binary_names {
            if let Some(path) = worktree.which(name) {
                self.cached_lsp_binary_path = Some(path.clone());
                return Ok(path);
            }
        }

        // Check cached path
        if let Some(ref path) = self.cached_lsp_binary_path
            && fs::metadata(path).is_ok_and(|m| m.is_file())
        {
            return Ok(path.clone());
        }

        // Check extension directory for previously downloaded binary (any version)
        let (platform, arch) = zed::current_platform();
        let binary_name = Self::get_platform_binary_name(platform, arch)?;

        // Look for existing downloaded version
        if let Ok(entries) = fs::read_dir(".") {
            for entry in entries.filter_map(Result::ok) {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                if name_str.starts_with("turbo-vsc-") {
                    let binary_path: String = format!("{name_str}/extension/out/{binary_name}");
                    if fs::metadata(&binary_path).is_ok_and(|m| m.is_file()) {
                        self.cached_lsp_binary_path = Some(binary_path.clone());
                        return Ok(binary_path);
                    }
                }
            }
        }

        // Step 5: Auto-download from VS Code Marketplace
        self.download_and_extract_binary(language_server_id)
            .map_err(|download_error| {
                // Download failed, show manual instructions
                zed::set_language_server_installation_status(
                    language_server_id,
                    &LanguageServerInstallationStatus::Failed(download_error.clone()),
                );

                format!(
                    r"turborepo-lsp auto-download failed: {download_error}

Manual installation options:

1. Build from source:
   ```sh
   git clone https://github.com/vercel/turborepo
   cd turborepo/crates/turborepo-lsp
   cargo build --release
   # Binary at target/release/turborepo-lsp
   ```

2. Extract from VS Code extension:
   - Install 'Turborepo' extension in VS Code
   - Find binary at ~/.vscode/extensions/vercel.turbo-vsc-*/out/{binary_name}

3. Configure path in Zed settings.json:
   ```json
{LSP_SETTINGS_EXAMPLE}
   ```
"
                )
            })
    }

    fn get_binary_names() -> Vec<&'static str> {
        vec![
            "turborepo-lsp",
            "turborepo-lsp-linux-x64",
            "turborepo-lsp-linux-arm64",
            "turborepo-lsp-darwin-x64",
            "turborepo-lsp-darwin-arm64",
        ]
    }

    fn get_platform_binary_name(platform: zed::Os, arch: zed::Architecture) -> Result<String> {
        let os = match platform {
            zed::Os::Mac => "darwin",
            zed::Os::Linux => "linux",
            zed::Os::Windows => "win32",
        };

        let cpu = match arch {
            zed::Architecture::Aarch64 => "arm64",
            zed::Architecture::X8664 => "x64",
            zed::Architecture::X86 => {
                return Err("32-bit x86 is not supported by turborepo-lsp".into());
            }
        };

        let ext = match platform {
            zed::Os::Windows => ".exe",
            _ => "",
        };

        Ok(format!("turborepo-lsp-{os}-{cpu}{ext}"))
    }

    /// Query VS Code Marketplace API to get VSIX download URL and version
    fn query_marketplace_vsix_url() -> Result<(String, String)> {
        let request_body = serde_json::json!({
            "filters": [{
                "criteria": [{
                    "filterType": 7,
                    "value": EXTENSION_ID
                }]
            }],
            "flags": 914
        });

        let request = HttpRequest {
            method: HttpMethod::Post,
            url: MARKETPLACE_API_URL.to_string(),
            headers: vec![
                ("Content-Type".to_string(), "application/json".to_string()),
                (
                    "Accept".to_string(),
                    "application/json;api-version=3.0-preview.1".to_string(),
                ),
            ],
            body: Some(request_body.to_string().into_bytes()),
            redirect_policy: zed_extension_api::http_client::RedirectPolicy::FollowAll,
        };

        let response = request
            .fetch()
            .map_err(|e| format!("Failed to query VS Code Marketplace: {e}"))?;

        let json: Value = serde_json::from_slice(&response.body)
            .map_err(|e| format!("Failed to parse marketplace response: {e}"))?;

        // Extract version
        let version = json["results"][0]["extensions"][0]["versions"][0]["version"]
            .as_str()
            .ok_or("Could not find extension version in marketplace response")?
            .to_string();

        // Extract VSIX URL from files array
        let files = json["results"][0]["extensions"][0]["versions"][0]["files"]
            .as_array()
            .ok_or("Could not find files array in marketplace response")?;

        let vsix_url = files
            .iter()
            .find(|f| {
                f["assetType"]
                    .as_str()
                    .is_some_and(|t| t == "Microsoft.VisualStudio.Services.VSIXPackage")
            })
            .and_then(|f| f["source"].as_str())
            .ok_or("Could not find VSIX download URL in marketplace response")?
            .to_string();

        Ok((vsix_url, version))
    }

    /// Download and extract the LSP binary from VS Code extension VSIX
    fn download_and_extract_binary(
        &mut self,
        language_server_id: &LanguageServerId,
    ) -> Result<String> {
        let (platform, arch) = zed::current_platform();
        let binary_name = Self::get_platform_binary_name(platform, arch)?;

        // Show downloading status
        zed::set_language_server_installation_status(
            language_server_id,
            &LanguageServerInstallationStatus::Downloading,
        );

        // Query marketplace for download URL
        let (vsix_url, version) = Self::query_marketplace_vsix_url()?;

        // Download destination - version-specific directory
        let download_dir = format!("turbo-vsc-{version}");
        let binary_path = format!("{download_dir}/extension/out/{binary_name}");

        // Check if already downloaded
        if fs::metadata(&binary_path).is_ok_and(|m| m.is_file()) {
            self.cached_lsp_binary_path = Some(binary_path.clone());
            return Ok(binary_path);
        }

        // Clean up old versions before downloading new one
        Self::cleanup_old_versions(&download_dir);

        // Download and extract VSIX (it's a ZIP file)
        zed::download_file(&vsix_url, &download_dir, DownloadedFileType::Zip)
            .map_err(|e| format!("Failed to download VSIX from marketplace: {e}"))?;

        // Make binary executable
        zed::make_file_executable(&binary_path)
            .map_err(|e| format!("Failed to make binary executable: {e}"))?;

        // Verify the binary exists after extraction
        if !fs::metadata(&binary_path).is_ok_and(|m| m.is_file()) {
            // List available binaries for better error message
            let out_dir = format!("{download_dir}/extension/out");
            let available = fs::read_dir(&out_dir).map_or_else(
                |_| "none found".to_string(),
                |entries| {
                    entries
                        .filter_map(Result::ok)
                        .filter_map(|e| e.file_name().into_string().ok())
                        .filter(|n| n.starts_with("turborepo-lsp-"))
                        .collect::<Vec<_>>()
                        .join(", ")
                },
            );

            return Err(format!(
                "Binary '{binary_name}' not found in VSIX.\n\
                Available binaries: {available}\n\
                Your platform: {platform:?} {arch:?}"
            ));
        }

        self.cached_lsp_binary_path = Some(binary_path.clone());
        Ok(binary_path)
    }

    /// Remove old version directories to save disk space
    fn cleanup_old_versions(current_version_dir: &str) {
        if let Ok(entries) = fs::read_dir(".") {
            for entry in entries.filter_map(Result::ok) {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                if name_str.starts_with("turbo-vsc-") && name_str != current_version_dir {
                    let _ = fs::remove_dir_all(entry.path());
                }
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
        // Check settings for custom path first
        if let Ok(settings) = ContextServerSettings::for_project(MCP_SERVER_ID, project)
            && let Some(settings) = settings.settings
            && let Some(path) = settings.get("binary_path").and_then(|v| v.as_str())
            && fs::metadata(path).is_ok_and(|m| m.is_file())
        {
            return path.to_string();
        }

        // Check cached path
        if let Some(ref path) = self.cached_mcp_binary_path
            && fs::metadata(path).is_ok_and(|m| m.is_file())
        {
            return path.clone();
        }

        // Default to turbo-mcp in PATH - OS will resolve it
        // If not found, user will see clear error from Zed
        MCP_BINARY_NAME.to_string()
    }

    /// Ensure the turbo daemon is running (required by turborepo-lsp)
    ///
    /// The LSP connects to the turbod daemon via a Unix socket. If the daemon
    /// isn't running, the LSP fails with "Internal error". This matches the
    /// VS Code extension behavior which starts the daemon before the LSP.
    fn ensure_daemon_running(worktree: &zed::Worktree) {
        // Find turbo binary in PATH
        if let Some(turbo_path) = worktree.which("turbo") {
            // Start daemon - this returns quickly if already running
            // Ignore result: best-effort, don't fail LSP startup if this fails
            let _ = Command::new(&turbo_path).args(["daemon", "start"]).output();
        }
        // If turbo not found, silently skip - user may have installed LSP separately
        // or the daemon may already be running from another source
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
        // Ensure turbo daemon is running (required by turborepo-lsp)
        Self::ensure_daemon_running(worktree);

        let binary_path = self.language_server_binary_path(language_server_id, worktree)?;

        // turborepo-lsp runs standalone with no arguments
        let args = if let Ok(lsp_settings) = LspSettings::for_worktree("turborepo-lsp", worktree) {
            lsp_settings
                .binary
                .and_then(|b| b.arguments)
                .unwrap_or_default()
        } else {
            vec![]
        };

        Ok(zed::Command {
            command: binary_path,
            args,
            env: Vec::default(),
        })
    }

    fn label_for_completion(
        &self,
        _language_server_id: &LanguageServerId,
        completion: zed::lsp::Completion,
    ) -> Option<zed::CodeLabel> {
        let label = &completion.label;

        // For package#task completions, highlight the parts
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
            // Simple task name
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
                .map_err(|e| e.to_string())?,
        }))
    }
}

zed::register_extension!(TurboExtension);

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    /// Extract keys from JSONC:
    /// - `// "key":` = commented-out key (single //)
    /// - `"key":` = active key
    /// - `/// ...` = doc comment (ignored)
    fn extract_jsonc_keys(input: &str) -> HashSet<String> {
        let mut keys = HashSet::new();
        for line in input.lines() {
            let trimmed = line.trim();
            // Skip doc comments (///)
            if trimmed.starts_with("///") {
                continue;
            }
            // Check for commented-out key: // "key":
            if let Some(rest) = trimmed.strip_prefix("//") {
                if let Some(key) = extract_key(rest.trim()) {
                    keys.insert(key);
                }
            }
            // Check for active key: "key":
            else if let Some(key) = extract_key(trimmed) {
                keys.insert(key);
            }
        }
        keys
    }

    /// Extract key from `"key": ...` pattern
    fn extract_key(s: &str) -> Option<String> {
        let s = s.trim();
        if s.starts_with('"') {
            if let Some(end) = s[1..].find('"') {
                let key = &s[1..=end];
                if s[end + 2..].trim_start().starts_with(':') {
                    return Some(key.to_string());
                }
            }
        }
        None
    }

    #[test]
    fn default_settings_keys_match_schema() {
        // Get schema properties
        let schema = schemars::schema_for!(TurboMcpSettings);
        let schema_json = serde_json::to_value(&schema).unwrap();
        let schema_props = schema_json["properties"]
            .as_object()
            .expect("schema should have properties");
        let schema_keys: HashSet<_> = schema_props.keys().cloned().collect();

        // Extract keys from default settings (both active and commented-out)
        let settings_keys = extract_jsonc_keys(DEFAULT_MCP_SETTINGS);

        // All keys in default settings must exist in schema
        for key in &settings_keys {
            assert!(
                schema_keys.contains(key),
                "default settings key '{key}' not found in schema. Schema keys: {schema_keys:?}"
            );
        }

        // All schema keys should appear in default settings
        for key in &schema_keys {
            assert!(
                settings_keys.contains(key),
                "schema key '{key}' not in default settings. Settings keys: {settings_keys:?}"
            );
        }
    }
}
