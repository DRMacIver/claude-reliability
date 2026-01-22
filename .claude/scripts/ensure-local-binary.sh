#!/usr/bin/env bash
# ensure-local-binary.sh - Ensures the claude-reliability binary is available locally
#
# This script:
# 1. In the source repo: rebuilds if source files changed
# 2. Otherwise checks for binary at .claude/bin/claude-reliability
# 3. If missing, tries to download from GitHub releases
# 4. Falls back to building from source if in the source repo
# 5. Prints the path to the binary on success, exits non-zero on failure

set -euo pipefail

REPO="DRMacIver/claude-reliability"
GITHUB_API="https://api.github.com"

# Get the project root (where CLAUDE.md and justfile live)
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

BINARY_PATH="${PROJECT_ROOT}/.claude/bin/claude-reliability"
MCP_BINARY_PATH="${PROJECT_ROOT}/.claude/bin/tasks-mcp"
PLUGIN_SOURCE_FILE="${PROJECT_ROOT}/.claude/plugin-source.txt"

# Check for external plugin source (installed from local checkout)
PLUGIN_SOURCE=""
if [[ -f "$PLUGIN_SOURCE_FILE" ]]; then
    PLUGIN_SOURCE="$(cat "$PLUGIN_SOURCE_FILE")"
    # Verify the plugin source still exists and is valid
    if [[ ! -f "${PLUGIN_SOURCE}/Cargo.toml" ]] || ! grep -q 'name = "claude-reliability"' "${PLUGIN_SOURCE}/Cargo.toml" 2>/dev/null; then
        PLUGIN_SOURCE=""  # Invalid, ignore it
    fi
fi

# ============================================================================
# Function definitions (must be before they're called)
# ============================================================================

# Check if we're in the source repository (has Cargo.toml with our crate name)
is_source_repo() {
    [[ -f "${PROJECT_ROOT}/Cargo.toml" ]] && grep -q 'name = "claude-reliability"' "${PROJECT_ROOT}/Cargo.toml" 2>/dev/null
}

# Check if we have a plugin source (either local repo or external source)
has_plugin_source() {
    is_source_repo || [[ -n "$PLUGIN_SOURCE" ]]
}

# Get the plugin source directory (local repo or external source)
get_plugin_source_dir() {
    if is_source_repo; then
        echo "$PROJECT_ROOT"
    elif [[ -n "$PLUGIN_SOURCE" ]]; then
        echo "$PLUGIN_SOURCE"
    else
        echo ""
    fi
}

# Check if source files are newer than either binary
source_is_newer() {
    local source_dir
    source_dir="$(get_plugin_source_dir)"

    if [[ -z "$source_dir" ]]; then
        return 1  # No source, can't be newer
    fi

    # Need to build if either binary is missing
    if [[ ! -x "$BINARY_PATH" ]] || [[ ! -x "$MCP_BINARY_PATH" ]]; then
        return 0  # Missing binary, need to build
    fi

    # Use the older of the two binaries for comparison
    local ref_binary="$BINARY_PATH"
    if [[ "$MCP_BINARY_PATH" -ot "$BINARY_PATH" ]]; then
        ref_binary="$MCP_BINARY_PATH"
    fi

    # Check if any Rust source files are newer than the reference binary
    local newer_files
    newer_files=$(find "${source_dir}/src" -name '*.rs' -newer "$ref_binary" 2>/dev/null | head -1)
    if [[ -n "$newer_files" ]]; then
        return 0  # Source is newer
    fi

    # Also check Cargo.toml for dependency changes
    if [[ "${source_dir}/Cargo.toml" -nt "$ref_binary" ]]; then
        return 0
    fi

    # Check templates directory too
    if [[ -d "${source_dir}/templates" ]]; then
        newer_files=$(find "${source_dir}/templates" -type f -newer "$ref_binary" 2>/dev/null | head -1)
        if [[ -n "$newer_files" ]]; then
            return 0
        fi
    fi

    return 1  # Binaries are up to date
}

# Detect platform
detect_platform() {
    local os arch
    os="$(uname -s)"
    arch="$(uname -m)"

    case "$os" in
        Linux)
            case "$arch" in
                x86_64|amd64) echo "linux-x86_64" ;;
                arm64|aarch64) echo "linux-aarch64" ;;
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
        echo "Could not determine latest version from GitHub API" >&2
        echo "URL: ${GITHUB_API}/repos/${REPO}/releases/latest" >&2
        return 1
    fi

    local version_num="${version#v}"
    local download_url="https://github.com/${REPO}/releases/download/${version}/claude-reliability-${version_num}-${artifact_name}.tar.gz"

    echo "Downloading version ${version}..." >&2

    local tmp_dir
    tmp_dir=$(mktemp -d)
    trap "rm -rf '$tmp_dir'" RETURN

    if command -v curl >/dev/null 2>&1; then
        if ! curl -fsSL "$download_url" -o "${tmp_dir}/release.tar.gz" 2>&1; then
            echo "Download failed from: ${download_url}" >&2
            return 1
        fi
    elif command -v wget >/dev/null 2>&1; then
        if ! wget -q "$download_url" -O "${tmp_dir}/release.tar.gz" 2>&1; then
            echo "Download failed from: ${download_url}" >&2
            return 1
        fi
    else
        echo "Neither curl nor wget available for download" >&2
        return 1
    fi

    mkdir -p "${PROJECT_ROOT}/.claude/bin"
    tar -xzf "${tmp_dir}/release.tar.gz" -C "$tmp_dir"
    mv "${tmp_dir}/claude-reliability" "$BINARY_PATH"
    chmod +x "$BINARY_PATH"

    # Also install tasks-mcp if present in the archive
    if [[ -f "${tmp_dir}/tasks-mcp" ]]; then
        mv "${tmp_dir}/tasks-mcp" "$MCP_BINARY_PATH"
        chmod +x "$MCP_BINARY_PATH"
        echo "MCP server installed to ${MCP_BINARY_PATH}" >&2
    fi

    return 0
}

# Install Rust using rustup
install_rust() {
    echo "Installing Rust..." >&2

    if command -v curl >/dev/null 2>&1; then
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y >&2
    elif command -v wget >/dev/null 2>&1; then
        wget -qO- https://sh.rustup.rs | sh -s -- -y >&2
    else
        echo "Neither curl nor wget available - cannot install Rust" >&2
        return 1
    fi

    # Source the cargo env to make it available in this session
    if [[ -f "${HOME}/.cargo/env" ]]; then
        # shellcheck source=/dev/null
        source "${HOME}/.cargo/env"
    fi

    if command -v cargo >/dev/null 2>&1; then
        echo "Rust installed successfully" >&2
        return 0
    fi

    echo "Rust installation failed" >&2
    return 1
}

# Try to build from source
build_from_source() {
    local source_dir
    source_dir="$(get_plugin_source_dir)"

    if [[ -z "$source_dir" ]]; then
        echo "No source directory available for building" >&2
        return 1
    fi

    echo "Building claude-reliability from source (${source_dir})..." >&2

    # Install Rust if not available
    if ! command -v cargo >/dev/null 2>&1; then
        echo "cargo not found - attempting to install Rust..." >&2
        if ! install_rust; then
            return 1
        fi
    fi

    # Build CLI binary
    if ! cargo build --release --features cli --manifest-path "${source_dir}/Cargo.toml" >&2; then
        echo "CLI build failed" >&2
        return 1
    fi

    # Build MCP server binary
    if ! cargo build --release --features mcp --manifest-path "${source_dir}/Cargo.toml" >&2; then
        echo "MCP server build failed" >&2
        return 1
    fi

    # Copy the built binaries to the project's .claude/bin
    mkdir -p "${PROJECT_ROOT}/.claude/bin"

    local built_binary="${source_dir}/target/release/claude-reliability"
    if [[ -x "$built_binary" ]]; then
        cp "$built_binary" "$BINARY_PATH"
        chmod +x "$BINARY_PATH"
    else
        echo "Built CLI binary not found at ${built_binary}" >&2
        return 1
    fi

    local built_mcp="${source_dir}/target/release/tasks-mcp"
    if [[ -x "$built_mcp" ]]; then
        cp "$built_mcp" "$MCP_BINARY_PATH"
        chmod +x "$MCP_BINARY_PATH"
        echo "MCP server built and installed" >&2
    else
        echo "Built MCP binary not found at ${built_mcp}" >&2
        return 1
    fi

    return 0
}

# ============================================================================
# Main logic
# ============================================================================

# Helper to check if both binaries are ready
binaries_ready() {
    [[ -x "$BINARY_PATH" ]] && "$BINARY_PATH" version >/dev/null 2>&1 && [[ -x "$MCP_BINARY_PATH" ]]
}

# Check if we have a plugin source and need to rebuild due to source changes
if has_plugin_source && source_is_newer; then
    echo "Source files changed, rebuilding..." >&2
    if build_from_source; then
        if binaries_ready; then
            echo "$BINARY_PATH"
            exit 0
        fi
    fi
    echo "Rebuild failed, trying other methods..." >&2
fi

# Check if both binaries exist and are executable
if binaries_ready; then
    echo "$BINARY_PATH"
    exit 0
fi

# Remove broken binaries
if [[ -x "$BINARY_PATH" ]] && ! "$BINARY_PATH" version >/dev/null 2>&1; then
    rm -f "$BINARY_PATH"
fi

# Not in source repo or missing binaries - try downloading
artifact_name=$(detect_platform)

if [[ -n "$artifact_name" ]]; then
    # Try downloading first
    if download_from_release "$artifact_name"; then
        if binaries_ready; then
            echo "$BINARY_PATH"
            exit 0
        fi
    fi
fi

# Fall back to building from source (only works if we have plugin source)
if has_plugin_source && build_from_source; then
    if binaries_ready; then
        echo "$BINARY_PATH"
        exit 0
    fi
fi

echo "ERROR: Failed to obtain claude-reliability binaries" >&2
echo "" >&2
echo "Tried:" >&2
echo "  - Download from GitHub releases (${GITHUB_API}/repos/${REPO}/releases/latest)" >&2
if has_plugin_source; then
    echo "  - Build from source ($(get_plugin_source_dir))" >&2
fi
echo "" >&2
echo "Expected binary locations:" >&2
echo "  CLI: ${BINARY_PATH}" >&2
echo "  MCP: ${MCP_BINARY_PATH}" >&2
echo "Detected platform: $(uname -s) $(uname -m)" >&2
echo "Supported platforms: Linux x86_64, Linux ARM64, macOS ARM64" >&2
echo "" >&2
echo "To build manually:" >&2
echo "  cd <source-dir>" >&2
echo "  cargo build --release --features cli" >&2
echo "  cargo build --release --features mcp" >&2
exit 1
