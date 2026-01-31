#!/bin/bash
# Ensure turbo daemon is running before starting LSP
turbo daemon start >/dev/null 2>&1
exec /home/kjanat/projects/zed-turborepo/target/release/turbo-lsp "$@"
