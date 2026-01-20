#!/usr/bin/env bash
# ensure-local-binary.sh - Ensures the claude-reliability binary is available locally
#
# This script:
# 1. Checks for binary at plugin-root/bin/claude-reliability
# 2. If missing, tries to download from GitHub releases
# 3. Prints the path to the binary on success, exits non-zero on failure

set -euo pipefail

REPO="DRMacIver/claude-reliability"
GITHUB_API="https://api.github.com"

# Get the plugin root
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PLUGIN_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

BINARY_PATH="${PLUGIN_ROOT}/bin/claude-reliability"

# Check if binary exists and is executable
if [[ -x "$BINARY_PATH" ]]; then
    # Verify it works
    if "$BINARY_PATH" version >/dev/null 2>&1; then
        echo "$BINARY_PATH"
        exit 0
    fi
    # Binary is broken, remove it
    rm -f "$BINARY_PATH"
fi

# Detect platform
detect_platform() {
    local os arch
    os="$(uname -s)"
    arch="$(uname -m)"

    case "$os" in
        Linux)
            case "$arch" in
                x86_64|amd64) echo "linux-x86_64" ;;
                *) echo "" ;;
            esac
            ;;
        Darwin)
            case "$arch" in
                arm64|aarch64) echo "macos-arm64" ;;
                *) echo "" ;;
            esac
            ;;
        *) echo "" ;;
    esac
}

# Try to download from GitHub releases
download_from_release() {
    local artifact_name="$1"

    echo "Downloading claude-reliability from GitHub releases..." >&2

    # Get latest version
    local version
    if command -v curl >/dev/null 2>&1; then
        version=$(curl -fsSL "${GITHUB_API}/repos/${REPO}/releases/latest" 2>/dev/null | grep '"tag_name"' | sed -E 's/.*"tag_name": *"([^"]+)".*/\1/' || echo "")
    elif command -v wget >/dev/null 2>&1; then
        version=$(wget -qO- "${GITHUB_API}/repos/${REPO}/releases/latest" 2>/dev/null | grep '"tag_name"' | sed -E 's/.*"tag_name": *"([^"]+)".*/\1/' || echo "")
    fi

    if [[ -z "$version" ]]; then
        echo "Could not determine latest version" >&2
        return 1
    fi

    local version_num="${version#v}"
    local download_url="https://github.com/${REPO}/releases/download/${version}/claude-reliability-${version_num}-${artifact_name}.tar.gz"

    echo "Downloading version ${version}..." >&2

    local tmp_dir
    tmp_dir=$(mktemp -d)
    trap "rm -rf '$tmp_dir'" RETURN

    if command -v curl >/dev/null 2>&1; then
        curl -fsSL "$download_url" -o "${tmp_dir}/release.tar.gz" || return 1
    elif command -v wget >/dev/null 2>&1; then
        wget -q "$download_url" -O "${tmp_dir}/release.tar.gz" || return 1
    else
        return 1
    fi

    mkdir -p "${PLUGIN_ROOT}/bin"
    tar -xzf "${tmp_dir}/release.tar.gz" -C "$tmp_dir"
    mv "${tmp_dir}/claude-reliability" "$BINARY_PATH"
    chmod +x "$BINARY_PATH"

    return 0
}

# Main logic
artifact_name=$(detect_platform)

if [[ -n "$artifact_name" ]]; then
    # Try downloading first
    if download_from_release "$artifact_name"; then
        if [[ -x "$BINARY_PATH" ]] && "$BINARY_PATH" version >/dev/null 2>&1; then
            echo "$BINARY_PATH"
            exit 0
        fi
    fi
fi

echo "ERROR: Failed to obtain claude-reliability binary" >&2
echo "Supported platforms: Linux x86_64, macOS ARM64" >&2
exit 1
