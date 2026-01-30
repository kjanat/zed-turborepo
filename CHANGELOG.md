# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- GitHub Actions CI/CD workflows for testing and releases
- Missing turbo.json syntax highlighting keys (`experimentalUI`,
  `legacyExperiments`, `allowAllOutputLogsOnSuccess`, `taskOverrides`)

### Fixed

- Repository URL in extension.toml

## [2.8.0] - 2025-01-30

### Added

- **turbo-mcp**: New MCP (Model Context Protocol) server for AI assistants
  - Resources: `turbo://config`, `turbo://tasks`, `turbo://packages`
  - Tools: `workdir` (get/set), `daemon` (status/start/stop), `run`, `graph`
  - Embedded turbo icon for MCP clients
- Complete turbo.json syntax highlighting based on official Turborepo docs
  - All root keys, task keys, remote cache keys, boundaries, future flags
  - Special microsyntax: `$TURBO_*` constants, `^task` dependencies,
    `package#task`, `!` negation
- JSONC grammar support for turbo.json (comments allowed)

### Fixed

- Grammar registration changed from `json` to `jsonc` to match config
- Clippy lints in biome patches
- `schemars::Schema` paths in biome patches

### Changed

- turbo-mcp redesigned from 9 tools to 4 tools + 3 resources pattern

## [2.7.0] - 2025-01-29

### Added

- Initial release
- **turbo-zed**: Zed extension with language support and LSP integration
- **turbo-lsp**: Thin stdio wrapper around Vercel's `turborepo-lsp`
- Syntax highlighting for turbo.json
- Code completion, diagnostics, and code actions via LSP

[Unreleased]: https://github.com/kjanat/zed-turborepo/compare/v2.8.0...HEAD
[2.8.0]: https://github.com/kjanat/zed-turborepo/compare/v2.7.0...v2.8.0
[2.7.0]: https://github.com/kjanat/zed-turborepo/releases/tag/v2.7.0
