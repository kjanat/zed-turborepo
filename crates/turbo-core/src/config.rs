//! turbo.json configuration parsing and types

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// Parsed turbo.json configuration
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurboConfig {
    /// Schema URL (optional)
    #[serde(rename = "$schema", skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,

    /// Extends another config
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extends: Option<Vec<String>>,

    /// Global dependencies that affect all tasks
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub global_dependencies: Vec<String>,

    /// Global environment variables
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub global_env: Vec<String>,

    /// Global pass-through environment variables
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub global_pass_through_env: Vec<String>,

    /// Task definitions (modern format)
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub tasks: HashMap<String, TurboTask>,

    /// Pipeline definitions (legacy format, maps to tasks)
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub pipeline: HashMap<String, TurboTask>,

    /// UI mode
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ui: Option<String>,

    /// Daemon configuration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub daemon: Option<bool>,

    /// Cache directory
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_dir: Option<String>,

    /// Remote cache configuration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote_cache: Option<RemoteCacheConfig>,

    /// Path to the config file (not serialized)
    #[serde(skip)]
    pub path: Option<PathBuf>,
}

/// Task configuration
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurboTask {
    /// Task dependencies
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub depends_on: Vec<String>,

    /// Environment variables this task depends on
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub env: Vec<String>,

    /// Pass-through environment variables
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pass_through_env: Vec<String>,

    /// Output files/directories
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub outputs: Vec<String>,

    /// Input files/directories
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inputs: Vec<String>,

    /// Cache behavior
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache: Option<bool>,

    /// Persistent task (long-running)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub persistent: Option<bool>,

    /// Interactive task
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interactive: Option<bool>,

    /// Output mode
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_logs: Option<String>,
}

/// Remote cache configuration
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteCacheConfig {
    /// Whether remote cache is enabled
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,

    /// Signature validation
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<bool>,

    /// Preflight requests
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preflight: Option<bool>,

    /// Timeout in seconds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u64>,
}

impl TurboConfig {
    /// Find and load turbo.json starting from the given directory
    pub async fn find_and_load(start_dir: &Path) -> Result<Self> {
        let path = Self::find_config_path(start_dir)?;
        Self::load(&path).await
    }

    /// Find turbo.json by walking up the directory tree
    pub fn find_config_path(start_dir: &Path) -> Result<PathBuf> {
        let mut current = start_dir.to_path_buf();
        loop {
            for name in ["turbo.json", "turbo.jsonc"] {
                let path = current.join(name);
                if path.exists() {
                    return Ok(path);
                }
            }
            if !current.pop() {
                break;
            }
        }
        Err(Error::ConfigNotFound(start_dir.to_path_buf()))
    }

    /// Load and parse turbo.json from a specific path
    pub async fn load(path: &Path) -> Result<Self> {
        let content = tokio::fs::read_to_string(path)
            .await
            .map_err(|e| Error::ReadFile {
                path: path.to_path_buf(),
                source: e,
            })?;

        Self::parse(&content, Some(path.to_path_buf()))
    }

    /// Load synchronously
    pub fn load_sync(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path).map_err(|e| Error::ReadFile {
            path: path.to_path_buf(),
            source: e,
        })?;

        Self::parse(&content, Some(path.to_path_buf()))
    }

    /// Parse turbo.json content (handles JSONC comments)
    pub fn parse(content: &str, path: Option<PathBuf>) -> Result<Self> {
        let stripped = strip_json_comments(content);

        let mut config: Self = serde_json::from_str(&stripped).map_err(|e| Error::ParseJson {
            path: path.clone().unwrap_or_default(),
            message: e.to_string(),
        })?;

        config.path = path;
        Ok(config)
    }

    /// Get all task names (from both `tasks` and legacy `pipeline`)
    pub fn task_names(&self) -> Vec<&str> {
        self.tasks
            .keys()
            .chain(self.pipeline.keys())
            .map(String::as_str)
            .collect()
    }

    /// Get a task by name (checks both `tasks` and `pipeline`)
    pub fn get_task(&self, name: &str) -> Option<&TurboTask> {
        self.tasks.get(name).or_else(|| self.pipeline.get(name))
    }

    /// Get cache directory (with default fallback)
    pub fn cache_dir(&self) -> &str {
        self.cache_dir
            .as_deref()
            .unwrap_or("node_modules/.cache/turbo")
    }
}

/// Strip JSON comments (// and /* */) from content
pub fn strip_json_comments(input: &str) -> String {
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
                    // Line comment
                    chars.next();
                    for next in chars.by_ref() {
                        if next == '\n' {
                            result.push('\n');
                            break;
                        }
                    }
                }
                Some('*') => {
                    // Block comment
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_line_comments() {
        let input = r#"{"key": "value" // comment
}"#;
        let result = strip_json_comments(input);
        assert!(!result.contains("//"));
        assert!(result.contains('\n'));
    }

    #[test]
    fn test_strip_block_comments() {
        let input = r#"{"key": "value" /* comment */}"#;
        let result = strip_json_comments(input);
        assert!(!result.contains("/*"));
    }

    #[test]
    fn test_preserves_urls_in_strings() {
        let input = r#"{"url": "https://example.com"}"#;
        assert_eq!(strip_json_comments(input), input);
    }

    #[test]
    fn test_parse_minimal_config() {
        let content = r#"{"tasks": {"build": {"outputs": ["dist/**"]}}}"#;
        let config = TurboConfig::parse(content, None).unwrap();
        assert!(config.tasks.contains_key("build"));
    }

    #[test]
    fn test_parse_legacy_pipeline() {
        let content = r#"{"pipeline": {"build": {}}}"#;
        let config = TurboConfig::parse(content, None).unwrap();
        assert!(config.pipeline.contains_key("build"));
        assert!(config.get_task("build").is_some());
    }
}
