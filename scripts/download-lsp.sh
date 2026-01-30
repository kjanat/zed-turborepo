#!/usr/bin/env bash
# Download turborepo-lsp binary from VS Code extension
set -euo pipefail

# Detect platform
KERNEL_NAME=$(uname -s)
case "${KERNEL_NAME}" in
	Linux*) OS="linux" ;;
	Darwin*) OS="darwin" ;;
	MINGW* | MSYS* | CYGWIN*) OS="win32" ;;
	*)
		echo "Unsupported OS: ${KERNEL_NAME}"
		exit 1
		;;
esac

# Detect architecture
MACHINE=$(uname -m)
case "${MACHINE}" in
	x86_64 | amd64) ARCH="x64" ;;
	aarch64 | arm64) ARCH="arm64" ;;
	*)
		echo "Unsupported architecture: ${MACHINE}"
		exit 1
		;;
esac

BINARY_NAME="turborepo-lsp-${OS}-${ARCH}"
if [[ "${OS}" = "win32" ]]; then
	BINARY_NAME="${BINARY_NAME}.exe"
fi

echo "Downloading turborepo-lsp for ${OS}-${ARCH}..."

# Create temp directory
TEMP_DIR=$(mktemp -d)
trap 'rm -rf "$TEMP_DIR"' EXIT

# Query VS Code marketplace API to get the CDN download URL
echo "Querying VS Code marketplace..."
VSIX_URL=$(curl -sX POST "https://marketplace.visualstudio.com/_apis/public/gallery/extensionquery" \
	-H "Content-Type: application/json" \
	-H "Accept: application/json;api-version=3.0-preview.1" \
	-d '{"filters":[{"criteria":[{"filterType":7,"value":"vercel.turbo-vsc"}]}],"flags":914}' \
	| grep -o '"source":"[^"]*VSIXPackage[^"]*"' \
	| head -1 \
	| sed 's/"source":"//;s/"//')

if [[ -z "${VSIX_URL}" ]]; then
	{
		echo "Failed to get download URL from marketplace."
		echo ""
		echo "Alternative: Build from source"
		echo "  git clone https://github.com/vercel/turborepo"
		echo "  cd turborepo/crates/turborepo-lsp"
		echo "  cargo build --release"
	}
	exit 1
fi

echo "Downloading VSIX from CDN..."
if ! curl -fsSL "${VSIX_URL}" -o "${TEMP_DIR}/turbo-vsc.vsix"; then
	echo "Failed to download VSIX."
	exit 1
fi

# Extract binary from VSIX (it's a ZIP file)
echo "Extracting binary..."
unzip -q "${TEMP_DIR}/turbo-vsc.vsix" -d "${TEMP_DIR}/vsix"

BINARY_PATH="${TEMP_DIR}/vsix/extension/out/${BINARY_NAME}"
if [[ ! -f "${BINARY_PATH}" ]]; then
	echo "Binary not found in VSIX: ${BINARY_NAME}"
	echo "Available binaries:"
	shopt -s nullglob
	files=("${TEMP_DIR}/vsix/extension/out/"turborepo-lsp*)
	shopt -u nullglob
	if ((${#files[@]})); then
		printf '  %s\n' "${files[@]##*/}"
	else
		echo "  (none)"
	fi
	exit 1
fi

# Determine install location
INSTALL_DIR="${XDG_DATA_HOME:-${HOME}/.local}/bin"
mkdir -p "${INSTALL_DIR}"

INSTALL_PATH="${INSTALL_DIR}/turborepo-lsp"
cp "${BINARY_PATH}" "${INSTALL_PATH}"
chmod +x "${INSTALL_PATH}"

echo ""
echo "✓ Installed to: ${INSTALL_PATH}"
echo ""
if [[ ":${PATH}:" != *":${INSTALL_DIR}:"* ]]; then
	echo "Add to PATH or configure Zed settings:"
	echo ""
	echo '  {'
	echo '    "lsp": {'
	echo '      "turborepo-lsp": {'
	echo "        \"binary\": { \"path\": \"${INSTALL_PATH}\" }"
	echo '      }'
	echo '    }'
	echo '  }'
else
	echo "The binary is in your PATH. Restart Zed to activate."
fi
