export const DOC_LINKS = {
  configuration: "https://turbo.build/repo/docs/reference/configuration",
  tasks: "https://turbo.build/repo/docs/reference/configuration#tasks",
  extends: "https://turbo.build/repo/docs/reference/configuration#extends",
  globalDependencies: "https://turbo.build/repo/docs/reference/configuration#globaldependencies",
  globalEnv: "https://turbo.build/repo/docs/reference/configuration#globalenv",
  globalPassThroughEnv: "https://turbo.build/repo/docs/reference/configuration#globalpassthroughenv",
  ui: "https://turbo.build/repo/docs/reference/configuration#ui",
  daemon: "https://turbo.build/repo/docs/reference/configuration#daemon",
  cacheDir: "https://turbo.build/repo/docs/reference/configuration#cachedir",
  remoteCache: "https://turbo.build/repo/docs/reference/configuration#remote-caching",
  dependsOn: "https://turbo.build/repo/docs/reference/configuration#dependson",
  outputs: "https://turbo.build/repo/docs/reference/configuration#outputs",
  inputs: "https://turbo.build/repo/docs/reference/configuration#inputs",
  env: "https://turbo.build/repo/docs/reference/configuration#env",
  passThroughEnv: "https://turbo.build/repo/docs/reference/configuration#passthroughenv",
  cache: "https://turbo.build/repo/docs/reference/configuration#cache",
  persistent: "https://turbo.build/repo/docs/reference/configuration#persistent",
  interactive: "https://turbo.build/repo/docs/reference/configuration#interactive",
  outputLogs: "https://turbo.build/repo/docs/reference/configuration#outputlogs",
} as const;

export const SCHEMA_KEYS = {
  topLevel: [
    "$schema",
    "tasks",
    "pipeline",
    "globalDependencies",
    "globalEnv",
    "globalPassThroughEnv",
    "ui",
    "daemon",
    "cacheDir",
    "extends",
    "remoteCache",
  ],
  taskFields: [
    "dependsOn",
    "outputs",
    "inputs",
    "env",
    "passThroughEnv",
    "cache",
    "persistent",
    "interactive",
    "outputLogs",
  ],
} as const;

export const TOP_LEVEL_HOVERS_JSON = String.raw`{
  "$schema": {
    "docsKey": "configuration",
    "context": "Use the official Turbo schema URL so editors understand the file shape.",
    "example": "{\n  \"$schema\": \"https://turborepo.dev/schema.json\"\n}"
  },
  "tasks": {
    "docsKey": "tasks",
    "context": "Each property inside tasks is a task name like build, lint, or test.",
    "example": "{\n  \"tasks\": {\n    \"build\": { \"dependsOn\": [\"^build\"], \"outputs\": [\"dist/**\"] }\n  }\n}"
  },
  "pipeline": {
    "docsKey": "tasks",
    "summaryOverride": "Legacy alias for tasks.",
    "context": "Modern Turbo config uses tasks. Keep pipeline only for older configs you still need to support.",
    "example": "{\n  \"pipeline\": {\n    \"build\": { \"dependsOn\": [\"^build\"] }\n  }\n}"
  },
  "globalDependencies": {
    "docsKey": "globalDependencies",
    "context": "Good for shared env files, root config files, or anything every task truly depends on.",
    "example": "{\n  \"globalDependencies\": [\".env\", \"tsconfig.base.json\"]\n}"
  },
  "globalEnv": {
    "docsKey": "globalEnv",
    "context": "Use this when a variable affects outputs and should invalidate cache everywhere.",
    "example": "{\n  \"globalEnv\": [\"NODE_ENV\", \"API_URL\"]\n}"
  },
  "globalPassThroughEnv": {
    "docsKey": "globalPassThroughEnv",
    "context": "Use sparingly. If a variable changes outputs, prefer globalEnv instead.",
    "example": "{\n  \"globalPassThroughEnv\": [\"CI\"]\n}"
  },
  "ui": {
    "docsKey": "ui",
    "context": "Useful when you want streamed logs, TUI mode, or stable CI output behavior.",
    "example": "{\n  \"ui\": \"stream\"\n}"
  },
  "daemon": {
    "docsKey": "daemon",
    "context": "The daemon still matters for watch flows and LSP-related state, even though parts of it are deprecated.",
    "example": "{\n  \"daemon\": true\n}"
  },
  "cacheDir": {
    "docsKey": "cacheDir",
    "context": "Default is node_modules/.cache/turbo. Change only if your repo needs a custom layout.",
    "example": "{\n  \"cacheDir\": \".turbo/cache\"\n}"
  },
  "extends": {
    "docsKey": "extends",
    "context": "Common in package-level configs that inherit root behavior.",
    "example": "{\n  \"extends\": [\"//\"]\n}"
  },
  "remoteCache": {
    "docsKey": "remoteCache",
    "context": "Useful for CI and teams, but only if your repo actually uses a remote cache backend.",
    "example": "{\n  \"remoteCache\": { \"enabled\": true }\n}"
  }
}`;

export const TASK_FIELD_HOVERS_JSON = String.raw`{
  "dependsOn": {
    "docsKey": "dependsOn",
    "context": "Entries can target the same package, dependency packages via ^, or a specific package via package#task.",
    "example": "{\n  \"tasks\": {\n    \"build\": {\n      \"dependsOn\": [\"^build\", \"lint\", \"web#codegen\"]\n    }\n  }\n}"
  },
  "outputs": {
    "docsKey": "outputs",
    "context": "Correct outputs are critical for cache hits and artifact restore.",
    "example": "{\n  \"tasks\": {\n    \"build\": {\n      \"outputs\": [\"dist/**\", \".next/**\"]\n    }\n  }\n}"
  },
  "inputs": {
    "docsKey": "inputs",
    "context": "Use when the default file hashing scope is too broad.",
    "example": "{\n  \"tasks\": {\n    \"lint\": {\n      \"inputs\": [\"src/**\", \"eslint.config.js\"]\n    }\n  }\n}"
  },
  "env": {
    "docsKey": "env",
    "context": "Prefer this when env changes can change task output.",
    "example": "{\n  \"tasks\": {\n    \"build\": {\n      \"env\": [\"API_URL\"]\n    }\n  }\n}"
  },
  "passThroughEnv": {
    "docsKey": "passThroughEnv",
    "context": "Handy for non-output-affecting runtime variables.",
    "example": "{\n  \"tasks\": {\n    \"dev\": {\n      \"passThroughEnv\": [\"PORT\"]\n    }\n  }\n}"
  },
  "cache": {
    "docsKey": "cache",
    "context": "Long-running or purely side-effectful tasks often disable cache.",
    "example": "{\n  \"tasks\": {\n    \"dev\": {\n      \"cache\": false\n    }\n  }\n}"
  },
  "persistent": {
    "docsKey": "persistent",
    "context": "Good for watch servers that should stay alive.",
    "example": "{\n  \"tasks\": {\n    \"dev\": {\n      \"persistent\": true\n    }\n  }\n}"
  },
  "interactive": {
    "docsKey": "interactive",
    "context": "Useful for prompts or TUI-style local workflows.",
    "example": "{\n  \"tasks\": {\n    \"dev\": {\n      \"interactive\": true\n    }\n  }\n}"
  },
  "outputLogs": {
    "docsKey": "outputLogs",
    "context": "Tune this when CI noise or replay behavior gets annoying.",
    "example": "{\n  \"tasks\": {\n    \"test\": {\n      \"outputLogs\": \"new-only\"\n    }\n  }\n}"
  }
}`;
