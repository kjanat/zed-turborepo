//! Package and task discovery for Turborepo workspaces

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Stdio;

use serde::{Deserialize, Serialize};

use crate::config::TurboConfig;
use crate::error::{Error, Result};

/// Discovered package information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Package {
    /// Package name from package.json
    pub name: String,
    /// Path to package directory
    pub path: PathBuf,
    /// Path to package.json
    pub package_json_path: PathBuf,
    /// Scripts defined in package.json
    #[serde(default)]
    pub scripts: HashMap<String, String>,
}

/// Task information combining turbo.json and package.json data
#[derive(Debug, Clone)]
pub struct TaskInfo {
    /// Task name (e.g., "build", "test")
    pub name: String,
    /// Packages that have this task as a script
    pub packages: Vec<String>,
    /// Task configuration from turbo.json (if defined)
    pub config: Option<crate::config::TurboTask>,
}

/// Package and task discovery
pub struct PackageDiscovery {
    /// Root directory of the monorepo
    root: PathBuf,
    /// Turbo configuration (if loaded)
    config: Option<TurboConfig>,
}

impl PackageDiscovery {
    /// Create a new discovery instance
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            config: None,
        }
    }

    /// Create with pre-loaded config
    pub fn with_config(root: impl Into<PathBuf>, config: TurboConfig) -> Self {
        Self {
            root: root.into(),
            config: Some(config),
        }
    }

    /// Get or load the turbo config
    pub async fn config(&mut self) -> Result<&TurboConfig> {
        if self.config.is_none() {
            self.config = Some(TurboConfig::find_and_load(&self.root).await?);
        }
        Ok(self.config.as_ref().unwrap())
    }

    /// Discover all packages in the workspace
    pub async fn discover_packages(&self) -> Result<Vec<Package>> {
        // Try `turbo ls --output json` first
        if let Ok(packages) = self.discover_via_turbo().await {
            return Ok(packages);
        }

        // Fallback to package.json workspaces
        self.discover_via_package_json().await
    }

    /// Discover packages using `turbo ls`
    async fn discover_via_turbo(&self) -> Result<Vec<Package>> {
        let output = tokio::process::Command::new("turbo")
            .args(["ls", "--output", "json"])
            .current_dir(&self.root)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .map_err(|e| Error::CommandFailed {
                command: "turbo ls".into(),
                message: e.to_string(),
            })?;

        if !output.status.success() {
            return Err(Error::CommandFailed {
                command: "turbo ls".into(),
                message: String::from_utf8_lossy(&output.stderr).into_owned(),
            });
        }

        let json_str = String::from_utf8_lossy(&output.stdout);

        // turbo ls output format: {"packages": [...]} or just array
        let packages: Vec<TurboLsPackage> =
            if let Ok(wrapper) = serde_json::from_str::<TurboLsOutput>(&json_str) {
                wrapper.packages
            } else {
                serde_json::from_str(&json_str).map_err(|e| Error::ParseJson {
                    path: PathBuf::from("turbo ls output"),
                    message: e.to_string(),
                })?
            };

        let mut result = Vec::with_capacity(packages.len());
        for pkg in packages {
            let pkg_path = self.root.join(&pkg.path);
            let package_json_path = pkg_path.join("package.json");

            let scripts = if package_json_path.exists() {
                Self::read_package_scripts(&package_json_path)
                    .await
                    .unwrap_or_default()
            } else {
                HashMap::new()
            };

            result.push(Package {
                name: pkg.name,
                path: pkg_path,
                package_json_path,
                scripts,
            });
        }

        Ok(result)
    }

    /// Discover packages via root package.json workspaces field
    async fn discover_via_package_json(&self) -> Result<Vec<Package>> {
        let root_pkg_path = self.root.join("package.json");
        let content = tokio::fs::read_to_string(&root_pkg_path)
            .await
            .map_err(|e| Error::ReadFile {
                path: root_pkg_path.clone(),
                source: e,
            })?;

        let root_pkg: serde_json::Value =
            serde_json::from_str(&content).map_err(|e| Error::ParseJson {
                path: root_pkg_path,
                message: e.to_string(),
            })?;

        let workspace_globs = match root_pkg.get("workspaces") {
            Some(serde_json::Value::Array(arr)) => arr
                .iter()
                .filter_map(|v| v.as_str())
                .map(String::from)
                .collect::<Vec<_>>(),
            Some(serde_json::Value::Object(obj)) => {
                // Yarn workspaces format: {"packages": [...]}
                obj.get("packages")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str())
                            .map(String::from)
                            .collect()
                    })
                    .unwrap_or_default()
            }
            _ => Vec::new(),
        };

        // For now, just return root package if no workspaces
        // TODO: implement glob expansion
        if workspace_globs.is_empty() {
            let scripts = Self::read_package_scripts(&self.root.join("package.json"))
                .await
                .unwrap_or_default();
            return Ok(vec![Package {
                name: root_pkg
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("root")
                    .to_string(),
                path: self.root.clone(),
                package_json_path: self.root.join("package.json"),
                scripts,
            }]);
        }

        // Simple glob handling - look for package.json in common locations
        let mut packages = Vec::new();
        for glob in &workspace_globs {
            let base = glob.trim_end_matches("/*").trim_end_matches("/**");
            let base_path = self.root.join(base);

            if base_path.is_dir() {
                if let Ok(mut entries) = tokio::fs::read_dir(&base_path).await {
                    while let Ok(Some(entry)) = entries.next_entry().await {
                        let pkg_json = entry.path().join("package.json");
                        if pkg_json.exists() {
                            if let Ok(pkg) = Self::load_package(&entry.path()).await {
                                packages.push(pkg);
                            }
                        }
                    }
                }
            }
        }

        Ok(packages)
    }

    /// Load a single package from its directory
    async fn load_package(path: &Path) -> Result<Package> {
        let package_json_path = path.join("package.json");
        let content = tokio::fs::read_to_string(&package_json_path)
            .await
            .map_err(|e| Error::ReadFile {
                path: package_json_path.clone(),
                source: e,
            })?;

        let pkg: serde_json::Value =
            serde_json::from_str(&content).map_err(|e| Error::ParseJson {
                path: package_json_path.clone(),
                message: e.to_string(),
            })?;

        let name = pkg
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();

        let scripts = pkg
            .get("scripts")
            .and_then(|v| v.as_object())
            .map(|obj| {
                obj.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect()
            })
            .unwrap_or_default();

        Ok(Package {
            name,
            path: path.to_path_buf(),
            package_json_path,
            scripts,
        })
    }

    /// Read scripts from a package.json
    async fn read_package_scripts(path: &Path) -> Result<HashMap<String, String>> {
        let content = tokio::fs::read_to_string(path)
            .await
            .map_err(|e| Error::ReadFile {
                path: path.to_path_buf(),
                source: e,
            })?;

        let pkg: serde_json::Value =
            serde_json::from_str(&content).map_err(|e| Error::ParseJson {
                path: path.to_path_buf(),
                message: e.to_string(),
            })?;

        Ok(pkg
            .get("scripts")
            .and_then(|v| v.as_object())
            .map(|obj| {
                obj.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect()
            })
            .unwrap_or_default())
    }

    /// Discover all tasks across packages
    pub async fn discover_tasks(&mut self) -> Result<Vec<TaskInfo>> {
        let config = self.config().await?.clone();
        let packages = self.discover_packages().await?;

        // Collect all unique task names from package scripts
        let mut task_packages: HashMap<String, Vec<String>> = HashMap::new();
        for pkg in &packages {
            for script_name in pkg.scripts.keys() {
                task_packages
                    .entry(script_name.clone())
                    .or_default()
                    .push(pkg.name.clone());
            }
        }

        // Also include tasks defined only in turbo.json
        for task_name in config.task_names() {
            task_packages.entry(task_name.to_string()).or_default();
        }

        // Build task info
        let mut tasks: Vec<TaskInfo> = task_packages
            .into_iter()
            .map(|(name, packages)| TaskInfo {
                config: config.get_task(&name).cloned(),
                name,
                packages,
            })
            .collect();

        tasks.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(tasks)
    }

    /// Get unique task names across all packages
    pub async fn task_names(&mut self) -> Result<HashSet<String>> {
        let tasks = self.discover_tasks().await?;
        Ok(tasks.into_iter().map(|t| t.name).collect())
    }
}

/// Internal: turbo ls JSON output format
#[derive(Deserialize)]
struct TurboLsOutput {
    packages: Vec<TurboLsPackage>,
}

/// Internal: package in turbo ls output
#[derive(Deserialize)]
struct TurboLsPackage {
    name: String,
    path: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_discovery_new() {
        let discovery = PackageDiscovery::new("/tmp");
        assert_eq!(discovery.root, PathBuf::from("/tmp"));
    }
}
