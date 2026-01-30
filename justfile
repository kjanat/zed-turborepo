set working-directory := "crates/turbo-zed"
set positional-arguments := true

# Display help
help:
    just -l

# Build the extension (debug)
build:
    cargo build

# Build the extension (release)
build-release:
    cargo build --release

# Check the extension compiles
check:
    cargo check

# Format code
fmt:
    cargo fmt

# Format code (nightly with import grouping)
fmt-nightly:
    rustup run nightly cargo fmt -- --config imports_granularity=Item

# Run clippy
clippy:
    cargo clippy --all-features

# Fix clippy warnings
fix *args:
    cargo clippy --fix --all-features --allow-dirty "$@"

# Install dependencies
install:
    rustup show active-toolchain
    cargo fetch

# Clean build artifacts
clean:
    cargo clean

# Update dependencies
update:
    cargo update

# Show extension structure
tree:
    tree -I 'target|grammars' .

# Download turborepo-lsp binary from VS Code marketplace
download-lsp:
    ./scripts/download-lsp.sh
