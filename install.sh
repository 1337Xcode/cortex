#!/bin/sh
# Cortex installer
# Detects OS and architecture, downloads the correct binary from GitHub Releases.
# Usage: curl -fsSL https://raw.githubusercontent.com/1337Xcode/cortex/main/install.sh | sh

set -e

REPO="1337Xcode/cortex"
INSTALL_DIR="${CORTEX_INSTALL_DIR:-$HOME/.local/bin}"

# Detect OS
case "$(uname -s)" in
    Linux*)   OS=linux;;
    Darwin*)  OS=darwin;;
    MINGW*|MSYS*|CYGWIN*) OS=win32;;
    *)        echo "Unsupported OS: $(uname -s)"; exit 1;;
esac

# Detect architecture
case "$(uname -m)" in
    x86_64|amd64)  ARCH=x64;;
    aarch64|arm64) ARCH=arm64;;
    *)             echo "Unsupported architecture: $(uname -m)"; exit 1;;
esac

BINARY="cortex"
if [ "$OS" = "win32" ]; then
    BINARY="cortex.exe"
fi

ARCHIVE="cortex-${OS}-${ARCH}.tar.gz"
URL="https://github.com/${REPO}/releases/latest/download/${ARCHIVE}"

echo "Downloading cortex for ${OS}-${ARCH}..."
echo "  From: ${URL}"

# Create install directory
mkdir -p "$INSTALL_DIR"

# Download and extract
if command -v curl >/dev/null 2>&1; then
    curl -fsSL "$URL" | tar -xz -C "$INSTALL_DIR"
elif command -v wget >/dev/null 2>&1; then
    wget -qO- "$URL" | tar -xz -C "$INSTALL_DIR"
else
    echo "Error: need curl or wget"
    exit 1
fi

# Make executable
chmod +x "$INSTALL_DIR/$BINARY"

echo ""
echo "Installed cortex to $INSTALL_DIR/$BINARY"
echo ""

# Check if install dir is in PATH
case ":$PATH:" in
    *":$INSTALL_DIR:"*) ;;
    *)
        echo "Add this to your shell profile:"
        echo "  export PATH=\"$INSTALL_DIR:\$PATH\""
        echo ""
        ;;
esac

echo "Next steps:"
echo "  cd /your/project"
echo "  cortex index"
echo "  cortex install"
echo "  cortex serve"
