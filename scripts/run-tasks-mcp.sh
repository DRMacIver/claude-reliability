#!/usr/bin/env bash
# run-tasks-mcp.sh - Wrapper to ensure tasks-mcp binary exists and run it
#
# This script ensures the tasks-mcp binary is available before running it.
# It uses the same caching strategy as ensure-binary.sh.

set -euo pipefail

REPO="DRMacIver/claude-reliability"
BINARY_NAME="tasks-mcp"
CACHE_DIR="${HOME}/.claude-reliability"

# Get the directory where this script lives (plugin root)
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PLUGIN_ROOT="$(dirname "$SCRIPT_DIR")"

# Check if we're in the source repository (has Cargo.toml with our crate name)
is_source_repo() {
    [[ -f "${PLUGIN_ROOT}/Cargo.toml" ]] && grep -q 'name = "claude-reliability"' "${PLUGIN_ROOT}/Cargo.toml" 2>/dev/null
}

# Check if source files are newer than the binary
source_is_newer() {
    local binary_path="$1"

    if [[ ! -x "$binary_path" ]]; then
        return 0  # No binary, need to build
    fi

    # Check if any Rust source files are newer than the binary
    local newer_files
    newer_files=$(find "${PLUGIN_ROOT}/src" -name '*.rs' -newer "$binary_path" 2>/dev/null | head -1)
    if [[ -n "$newer_files" ]]; then
        return 0  # Source is newer
    fi

    # Also check Cargo.toml for dependency changes
    if [[ "${PLUGIN_ROOT}/Cargo.toml" -nt "$binary_path" ]]; then
        return 0
    fi

    return 1  # Binary is up to date
}

# Detect platform
detect_platform() {
    local os arch
    os="$(uname -s | tr '[:upper:]' '[:lower:]')"
    arch="$(uname -m)"

    case "$os" in
        linux) os="linux" ;;
        darwin) os="darwin" ;;
        *) echo "Unsupported OS: $os" >&2; return 1 ;;
    esac

    case "$arch" in
        x86_64|amd64) arch="x86_64" ;;
        aarch64|arm64) arch="aarch64" ;;
        *) echo "Unsupported architecture: $arch" >&2; return 1 ;;
    esac

    echo "${os}-${arch}"
}

# Get latest release version from GitHub
get_latest_version() {
    local url="https://api.github.com/repos/${REPO}/releases/latest"
    if command -v curl >/dev/null 2>&1; then
        curl -fsSL "$url" 2>/dev/null | grep '"tag_name"' | sed -E 's/.*"v?([^"]+)".*/\1/' || true
    elif command -v wget >/dev/null 2>&1; then
        wget -qO- "$url" 2>/dev/null | grep '"tag_name"' | sed -E 's/.*"v?([^"]+)".*/\1/' || true
    fi
}

# Map platform to release artifact name
get_artifact_name() {
    local platform="$1"
    case "$platform" in
        linux-x86_64) echo "linux-x86_64" ;;
        darwin-aarch64) echo "macos-arm64" ;;
        *) echo "" ;;
    esac
}

# Download binary from GitHub releases
download_binary() {
    local version="$1"
    local platform="$2"
    local target_path="$3"

    local artifact_name
    artifact_name="$(get_artifact_name "$platform")"
    if [[ -z "$artifact_name" ]]; then
        echo "No release available for platform: $platform" >&2
        return 1
    fi

    # Release format: claude-reliability-VERSION-ARTIFACT.tar.gz (contains both binaries)
    local tarball="claude-reliability-${version}-${artifact_name}.tar.gz"
    local url="https://github.com/${REPO}/releases/download/v${version}/${tarball}"

    mkdir -p "$(dirname "$target_path")"

    echo "Downloading ${BINARY_NAME} v${version} for ${artifact_name}..." >&2

    local tmp_dir
    tmp_dir="$(mktemp -d)"
    trap 'rm -rf "$tmp_dir"' RETURN

    local tarball_path="${tmp_dir}/release.tar.gz"

    if command -v curl >/dev/null 2>&1; then
        if ! curl -fsSL "$url" -o "$tarball_path" 2>/dev/null; then
            return 1
        fi
    elif command -v wget >/dev/null 2>&1; then
        if ! wget -q "$url" -O "$tarball_path" 2>/dev/null; then
            return 1
        fi
    else
        echo "Neither curl nor wget available" >&2
        return 1
    fi

    # Extract the tarball
    if ! tar -xzf "$tarball_path" -C "$tmp_dir" 2>/dev/null; then
        echo "Failed to extract tarball" >&2
        return 1
    fi

    # Move binary to target location
    if [[ -f "${tmp_dir}/${BINARY_NAME}" ]]; then
        mv "${tmp_dir}/${BINARY_NAME}" "$target_path"
        chmod +x "$target_path"
        return 0
    fi

    echo "Binary ${BINARY_NAME} not found in tarball" >&2
    return 1
}

# Build from source using cargo
build_from_source() {
    local target_path="$1"

    if ! command -v cargo >/dev/null 2>&1; then
        echo "cargo not found" >&2
        return 1
    fi

    if [[ ! -f "${PLUGIN_ROOT}/Cargo.toml" ]]; then
        echo "Cargo.toml not found in plugin directory" >&2
        return 1
    fi

    echo "Building ${BINARY_NAME} from source..." >&2

    if cargo build --release --features mcp --manifest-path "${PLUGIN_ROOT}/Cargo.toml" >&2; then
        local built_binary="${PLUGIN_ROOT}/target/release/${BINARY_NAME}"
        if [[ -x "$built_binary" ]]; then
            mkdir -p "$(dirname "$target_path")"
            cp "$built_binary" "$target_path"
            chmod +x "$target_path"
            return 0
        fi
    fi

    return 1
}

# Main logic
main() {
    local platform binary_path version

    # Detect platform
    if ! platform="$(detect_platform)"; then
        echo "ERROR: Could not detect platform" >&2
        exit 1
    fi

    binary_path="${CACHE_DIR}/bin/${BINARY_NAME}"

    # In the source repo, check if we need to rebuild due to source changes
    if is_source_repo && source_is_newer "$binary_path"; then
        echo "Source files changed, rebuilding ${BINARY_NAME}..." >&2
        if build_from_source "$binary_path"; then
            exec "$binary_path" "$@"
        fi
        echo "Rebuild failed, trying other methods..." >&2
    fi

    # Check if we have a cached binary
    if [[ -x "$binary_path" ]]; then
        exec "$binary_path" "$@"
    fi

    # Try to download from GitHub releases
    if ! is_source_repo; then
        version="$(get_latest_version)"
        if [[ -n "$version" ]]; then
            if download_binary "$version" "$platform" "$binary_path"; then
                exec "$binary_path" "$@"
            fi
        fi
    fi

    # Fall back to building from source
    if build_from_source "$binary_path"; then
        exec "$binary_path" "$@"
    fi

    # All methods failed
    echo "ERROR: Could not obtain ${BINARY_NAME} binary" >&2
    exit 1
}

main "$@"
