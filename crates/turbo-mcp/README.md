# turbo-mcp

An MCP (Model Context Protocol) server for Turborepo monorepos.

## Installation

```bash
cargo install --path .
```

Or download from [releases](https://github.com/kjanat/zed-turborepo/releases).

## Configuration

### Claude Desktop

Add to `~/.config/claude/claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "turbo": {
      "command": "turbo-mcp"
    }
  }
}
```

### Cursor

Add to `.cursor/mcp.json`:

```json
{
  "mcpServers": {
    "turbo": {
      "command": "turbo-mcp"
    }
  }
}
```

### Windsurf

Add to Windsurf MCP settings:

```json
{
  "mcpServers": {
    "turbo": {
      "command": "turbo-mcp"
    }
  }
}
```

## Resources

Read-only access to Turborepo configuration and state.

| URI                | Description                           |
| ------------------ | ------------------------------------- |
| `turbo://config`   | Full turbo.json configuration         |
| `turbo://tasks`    | List of defined tasks                 |
| `turbo://packages` | Workspace packages (from `turbo ls`)  |
| `turbo://cache`    | Cache configuration and daemon status |

## Tools

### workdir

Manage working directory.

```json
{"action": "get"}
{"action": "set", "path": "/path/to/project"}
```

### daemon

Control turbo daemon.

```json
{"action": "status"}
{"action": "start"}
{"action": "stop"}
```

### run

Execute turbo tasks.

```json
{
  "tasks": ["build", "test"],
  "filter": "@myapp/*",
  "dry_run": false,
  "continue_on_error": false
}
```

### graph

Show task dependency graph.

```json
{ "task": "build" }
```

### prune

Prune workspace to minimal subset for a package.

```json
{
  "scope": "@myapp/web",
  "out_dir": "out",
  "docker": true
}
```

### query

Query the task graph.

```json
{ "query": "Why does web depend on shared?" }
```

### lint

Run turbo lint to check configuration.

```json
{ "packages": ["@myapp/web"] }
```

### info

Get package or workspace info.

```json
{ "package": "@myapp/web" }
```

## License

MIT
