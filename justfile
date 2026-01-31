# Individually override working dir using: `[working-directory: 'bar']`

set working-directory := "crates/turbo-zed"
set positional-arguments := true
set unstable := true

# Display help
help:
    just --list --unsorted

# Build the extension (debug)
build:
    cargo build

# Build the extension (release)
build-release:
    cargo build --release

# Check the extension compiles
[working-directory('.')]
check:
    cargo check

# Format code
[working-directory('.')]
fmt:
    dprint fmt
    cargo +nightly fmt

# Run clippy
[working-directory('.')]
clippy:
    cargo +nightly clippy --all-features

# Fix clippy warnings
[working-directory('.')]
fix *args:
    cargo +nightly clippy --fix --all-features --allow-dirty "$@"

# Install dependencies
[working-directory('.')]
install:
    rustup show active-toolchain
    cargo fetch

# Clean build artifacts
[working-directory('.')]
clean:
    cargo clean

# Update dependencies
[working-directory('.')]
update:
    cargo update

# Show extension structure
[working-directory('.')]
tree:
    tree --gitignore

# Download turborepo-lsp binary from VS Code marketplace
[working-directory('.')]
download-lsp:
    ./scripts/download-lsp.sh

# Run all tests
[working-directory('.')]
test:
    cargo test --workspace

# Build turbo-mcp (debug)
[working-directory('.')]
build-mcp:
    cargo build -p turbo-mcp

# Build turbo-mcp (release)
[working-directory('.')]
build-mcp-release:
    cargo build -p turbo-mcp --release

# Install turbo-mcp locally
[working-directory('.')]
install-mcp:
    cargo install --path crates/turbo-mcp

# Build turbo-lsp (release)
[working-directory('.')]
build-lsp-release:
    cargo build -p turbo-lsp --release

# Install turbo-lsp locally
[working-directory('.')]
install-lsp:
    cargo install --path crates/turbo-lsp

# Package the Zed extension
package:
    cargo build --release --target wasm32-wasip2
