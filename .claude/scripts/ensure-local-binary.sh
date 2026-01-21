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

# Check if source files are newer than the binary
source_is_newer() {
    local source_dir
    source_dir="$(get_plugin_source_dir)"

    if [[ -z "$source_dir" ]]; then
        return 1  # No source, can't be newer
    fi

    if [[ ! -x "$BINARY_PATH" ]]; then
        return 0  # No binary, need to build
    fi

    # Check if any Rust source files are newer than the binary
    local newer_files
    newer_files=$(find "${source_dir}/src" -name '*.rs' -newer "$BINARY_PATH" 2>/dev/null | head -1)
    if [[ -n "$newer_files" ]]; then
        return 0  # Source is newer
    fi

    # Also check Cargo.toml for dependency changes
    if [[ "${source_dir}/Cargo.toml" -nt "$BINARY_PATH" ]]; then
        return 0
    fi

    # Check templates directory too
    if [[ -d "${source_dir}/templates" ]]; then
        newer_files=$(find "${source_dir}/templates" -type f -newer "$BINARY_PATH" 2>/dev/null | head -1)
        if [[ -n "$newer_files" ]]; then
            return 0
        fi
    fi

    return 1  # Binary is up to date
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

    mkdir -p "${PROJECT_ROOT}/.claude/bin"
    tar -xzf "${tmp_dir}/release.tar.gz" -C "$tmp_dir"
    mv "${tmp_dir}/claude-reliability" "$BINARY_PATH"
    chmod +x "$BINARY_PATH"

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

    # Build using cargo
    if ! cargo build --release --features cli --manifest-path "${source_dir}/Cargo.toml" >&2; then
        echo "Build failed" >&2
        return 1
    fi

    # Copy the built binary to the project's .claude/bin
    local built_binary="${source_dir}/target/release/claude-reliability"
    if [[ -x "$built_binary" ]]; then
        mkdir -p "${PROJECT_ROOT}/.claude/bin"
        cp "$built_binary" "$BINARY_PATH"
        chmod +x "$BINARY_PATH"
        return 0
    fi

    echo "Built binary not found at ${built_binary}" >&2
    return 1
}

# ============================================================================
# Main logic
# ============================================================================

# Check if we have a plugin source and need to rebuild due to source changes
if has_plugin_source && source_is_newer; then
    echo "Source files changed, rebuilding..." >&2
    if build_from_source; then
        if [[ -x "$BINARY_PATH" ]] && "$BINARY_PATH" version >/dev/null 2>&1; then
            echo "$BINARY_PATH"
            exit 0
        fi
    fi
    echo "Rebuild failed, trying other methods..." >&2
fi

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

# Not in source repo or no binary - try downloading
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

# Fall back to building from source (only works if we have plugin source)
if has_plugin_source && build_from_source; then
    if [[ -x "$BINARY_PATH" ]] && "$BINARY_PATH" version >/dev/null 2>&1; then
        echo "$BINARY_PATH"
        exit 0
    fi
fi

echo "ERROR: Failed to obtain claude-reliability binary" >&2
echo "Supported platforms: Linux x86_64, macOS ARM64" >&2
exit 1
