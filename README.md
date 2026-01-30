# Turbo Extension for Zed

Language support for [Turborepo](https://turbo.build/repo) `turbo.json`
configuration files with LSP integration and MCP server for AI assistants.

## Architecture

```
zed-turborepo/
├── crates/
│   ├── turbo-zed/     # Zed extension (WASM)
│   ├── turbo-lsp/     # Thin wrapper around turborepo-lsp
│   └── turbo-mcp/     # MCP server for AI assistants
└── patches/           # Biome crate patches
```

## Features

### Zed Extension (turbo-zed)

- **Syntax Highlighting**: Full syntax highlighting for `turbo.json` files with
  special highlighting for turbo-specific keys
- **Language Server**: Integration with turborepo-lsp providing:
  - Code completion for task and package names
  - Diagnostics for invalid configurations
  - Hover information
  - Go to definition/references
  - Code lens for running tasks
  - Quick fixes for deprecated syntax

### MCP Server (turbo-mcp)

An MCP (Model Context Protocol) server for AI assistants like Claude, Cursor,
and Windsurf.

#### Resources (Read-only)

| Resource           | Description                    |
| ------------------ | ------------------------------ |
| `turbo://config`   | Full turbo.json configuration  |
| `turbo://tasks`    | List of defined tasks          |
| `turbo://packages` | Workspace packages             |
| `turbo://cache`    | Cache configuration and status |

#### Tools (Actions)

| Tool      | Description                              |
| --------- | ---------------------------------------- |
| `workdir` | Get/set working directory                |
| `daemon`  | Control turbo daemon (status/start/stop) |
| `run`     | Execute turbo tasks                      |
| `graph`   | Show task dependency graph               |
| `prune`   | Prune workspace to minimal subset        |
| `query`   | Query the task graph                     |
| `lint`    | Run turbo lint                           |
| `info`    | Get package/workspace info               |

## Installation

### Zed Extension

Search for "Turbo" in the Zed extensions panel and install.

### MCP Server

```bash
# Build from source
cargo install --path crates/turbo-mcp

# Or download from releases
```

Configure in your AI client (e.g., Claude Desktop):

```json
{
  "mcpServers": {
    "turbo": {
      "command": "turbo-mcp"
    }
  }
}
```

## LSP Binary

The `turborepo-lsp` binary is **automatically downloaded** from the VS Code
Marketplace on first use. No manual installation required!

### Alternative Installation Methods

If auto-download fails or you prefer manual installation:

#### Option 1: Download Script

```bash
# From the extension directory
./scripts/download-lsp.sh

# Or using just
just download-lsp
```

This downloads the binary from the VS Code marketplace and installs it to
`~/.local/share/bin/turborepo-lsp`.

#### Option 2: Build from Source

```bash
git clone https://github.com/vercel/turborepo
cd turborepo/crates/turborepo-lsp
cargo build --release
# Binary at target/release/turborepo-lsp
```

#### Option 3: Extract from VS Code Extension

If you have VS Code with the Turborepo extension installed:

```bash
cp ~/.vscode/extensions/vercel.turbo-vsc-*/out/turborepo-lsp-linux-x64 ~/.local/bin/turborepo-lsp
chmod +x ~/.local/bin/turborepo-lsp
```

Platform binaries: `turborepo-lsp-{linux,darwin}-{x64,arm64}`

## Configuration

Configure the LSP binary path in your Zed settings (optional - only needed if
auto-download doesn't work):

```jsonc
{
  "lsp": {
    "turborepo-lsp": {
      "binary": {
        "path": "/path/to/turborepo-lsp",
      },
    },
  },
}
```

## LSP Capabilities

The Turborepo LSP provides:

| Feature          | Description                                                |
| ---------------- | ---------------------------------------------------------- |
| **Completion**   | Task names, package names, `package#task` combinations     |
| **References**   | Find scripts in package.json files matching pipeline tasks |
| **Code Lens**    | "Run task" commands above task definitions                 |
| **Code Actions** | Quick fixes for deprecated `$` env var syntax              |
| **Diagnostics**  | Validation errors for turbo.json                           |

### Diagnostics

| Code                            | Description                             |
| ------------------------------- | --------------------------------------- |
| `turbo:no-such-package`         | Referenced package doesn't exist        |
| `turbo:no-such-task`            | Referenced task doesn't exist           |
| `turbo:no-such-task-in-package` | Task doesn't exist in specified package |
| `turbo:self-dependency`         | Task depends on itself                  |
| `deprecated:env-var`            | `$` syntax is deprecated                |

## Development

```bash
# Build all crates
cargo build

# Run tests
cargo test

# Build release binaries
cargo build --release
```

## License

MIT
