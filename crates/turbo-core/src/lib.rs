//! Turbo Core
//!
//! Shared core functionality for Turborepo tooling (LSP, MCP, etc.)
//!
//! ## Modules
//! - [`config`] - turbo.json parsing and validation
//! - [`discovery`] - package and task discovery
//! - [`error`] - common error types

pub mod config;
pub mod discovery;
pub mod error;

pub use config::{TurboConfig, TurboTask};
pub use discovery::{Package, PackageDiscovery, TaskInfo};
pub use error::{Error, Result};
