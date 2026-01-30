use std::fs;

use zed_extension_api::{
    self as zed, DownloadedFileType, LanguageServerId, LanguageServerInstallationStatus, Result,
    http_client::{HttpMethod, HttpRequest},
    process::Command,
    serde_json::{self, Value},
    settings::LspSettings,
};

const MARKETPLACE_API_URL: &str =
    "https://marketplace.visualstudio.com/_apis/public/gallery/extensionquery";
const EXTENSION_ID: &str = "vercel.turbo-vsc";

struct TurboExtension {
    cached_binary_path: Option<String>,
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
                self.cached_binary_path = Some(path.clone());
                return Ok(path);
            }
        }

        // Check cached path
        if let Some(ref path) = self.cached_binary_path
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
                        self.cached_binary_path = Some(binary_path.clone());
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
                    "turborepo-lsp auto-download failed: {download_error}\n\n\
                    Manual installation options:\n\n\
                    1. Build from source:\n\
                       git clone https://github.com/vercel/turborepo\n\
                       cd turborepo/crates/turborepo-lsp\n\
                       cargo build --release\n\
                       # Binary at target/release/turborepo-lsp\n\n\
                    2. Extract from VS Code extension:\n\
                       - Install 'Turborepo' extension in VS Code\n\
                       - Find binary at ~/.vscode/extensions/vercel.turbo-vsc-*/out/{binary_name}\n\n\
                    3. Configure path in Zed settings.json:\n\
                       {{\n\
                         \"lsp\": {{\n\
                           \"turborepo-lsp\": {{\n\
                             \"binary\": {{ \"path\": \"/path/to/turborepo-lsp\" }}\n\
                           }}\n\
                         }}\n\
                       }}"
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
            self.cached_binary_path = Some(binary_path.clone());
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

        self.cached_binary_path = Some(binary_path.clone());
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
            cached_binary_path: None,
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
}

zed::register_extension!(TurboExtension);
