# Turborepo MCP Server

The Turborepo MCP server provides AI assistants with context about your
Turborepo monorepo.

## Features

- **Resources**: Access turbo.json config, task definitions, and workspace
  packages
- **Tools**: Control turbo daemon, run tasks, view dependency graphs

## Installation

### Option 1: Install via Cargo (Recommended)

```bash
cargo install turbo-mcp
```

### Option 2: Build from Source

```bash
git clone https://github.com/kjanat/zed-turborepo
cd zed-turborepo
cargo install --path crates/turbo-mcp
```

### Option 3: Custom Binary Path

Configure `binary_path` in the settings below.

## Requirements

- A Turborepo project with `turbo.json` in the workspace
- The `turbo` CLI installed and available in PATH (for daemon/run commands)

## Available Resources

| URI                | Description                    |
| ------------------ | ------------------------------ |
| `turbo://config`   | Full turbo.json configuration  |
| `turbo://tasks`    | List of defined tasks          |
| `turbo://packages` | Workspace packages             |
| `turbo://cache`    | Cache configuration and status |

## Available Tools

| Tool      | Description                              |
| --------- | ---------------------------------------- |
| `workdir` | Get or set working directory             |
| `daemon`  | Control turbo daemon (status/start/stop) |
| `run`     | Execute turbo tasks                      |
| `graph`   | Show task dependency graph               |
