#!/usr/bin/env bash
# install.sh - Install claude-reliability plugin into a project
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/DRMacIver/claude-reliability/main/scripts/install.sh | bash
#
# Or manually:
#   ./scripts/install.sh [--target-dir /path/to/project]
#
# This script:
# 1. Detects the platform (Linux x86_64 or macOS ARM64)
# 2. Fetches the latest release from GitHub
# 3. Downloads and extracts the binary to .claude/bin/
# 4. Copies hook scripts and settings to .claude/
#
# Requirements:
# - curl or wget
# - tar
# - GitHub API access (no auth required for public repos)

set -euo pipefail

REPO="DRMacIver/claude-reliability"
GITHUB_API="https://api.github.com"
RAW_URL="https://raw.githubusercontent.com/${REPO}/main"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

log_info() { echo -e "${GREEN}[INFO]${NC} $1"; }
log_warn() { echo -e "${YELLOW}[WARN]${NC} $1"; }
log_error() { echo -e "${RED}[ERROR]${NC} $1" >&2; }

# Detect platform
detect_platform() {
    local os arch artifact_name

    os="$(uname -s)"
    arch="$(uname -m)"

    case "$os" in
        Linux)
            case "$arch" in
                x86_64|amd64)
                    artifact_name="linux-x86_64"
                    ;;
                *)
                    log_error "Unsupported Linux architecture: $arch"
                    log_error "Currently supported: x86_64"
                    exit 1
                    ;;
            esac
            ;;
        Darwin)
            case "$arch" in
                arm64|aarch64)
                    artifact_name="macos-arm64"
                    ;;
                x86_64)
                    log_error "macOS x86_64 is not currently supported"
                    log_error "Please use macOS on Apple Silicon (arm64)"
                    exit 1
                    ;;
                *)
                    log_error "Unsupported macOS architecture: $arch"
                    exit 1
                    ;;
            esac
            ;;
        *)
            log_error "Unsupported operating system: $os"
            log_error "Currently supported: Linux, macOS"
            exit 1
            ;;
    esac

    echo "$artifact_name"
}

# Get latest release version from GitHub API
get_latest_version() {
    local version

    if command -v curl >/dev/null 2>&1; then
        version=$(curl -fsSL "${GITHUB_API}/repos/${REPO}/releases/latest" | grep '"tag_name"' | sed -E 's/.*"tag_name": *"([^"]+)".*/\1/')
    elif command -v wget >/dev/null 2>&1; then
        version=$(wget -qO- "${GITHUB_API}/repos/${REPO}/releases/latest" | grep '"tag_name"' | sed -E 's/.*"tag_name": *"([^"]+)".*/\1/')
    else
        log_error "Neither curl nor wget found. Please install one of them."
        exit 1
    fi

    if [[ -z "$version" ]]; then
        log_error "Failed to fetch latest version from GitHub"
        exit 1
    fi

    echo "$version"
}

# Download a file
download() {
    local url="$1"
    local output="$2"

    if command -v curl >/dev/null 2>&1; then
        curl -fsSL "$url" -o "$output"
    elif command -v wget >/dev/null 2>&1; then
        wget -q "$url" -O "$output"
    fi
}

# Download and extract the binary
install_binary() {
    local target_dir="$1"
    local artifact_name="$2"
    local version="$3"
    local version_num="${version#v}"  # Remove 'v' prefix

    local download_url="https://github.com/${REPO}/releases/download/${version}/claude-reliability-${version_num}-${artifact_name}.tar.gz"
    local bin_dir="${target_dir}/.claude/bin"
    local tmp_dir

    tmp_dir=$(mktemp -d)
    trap "rm -rf '$tmp_dir'" EXIT

    log_info "Downloading claude-reliability ${version} for ${artifact_name}..."
    download "$download_url" "${tmp_dir}/release.tar.gz"

    log_info "Extracting binary..."
    mkdir -p "$bin_dir"
    tar -xzf "${tmp_dir}/release.tar.gz" -C "$tmp_dir"
    mv "${tmp_dir}/claude-reliability" "${bin_dir}/claude-reliability"
    chmod +x "${bin_dir}/claude-reliability"

    # Verify the binary works
    if ! "${bin_dir}/claude-reliability" version >/dev/null 2>&1; then
        log_error "Binary verification failed"
        exit 1
    fi

    log_info "Binary installed to ${bin_dir}/claude-reliability"
}

# Download hook scripts from the repository
install_scripts() {
    local target_dir="$1"
    local scripts_dir="${target_dir}/.claude/scripts"
    local hooks_dir="${scripts_dir}/hooks"
    local commands_dir="${target_dir}/.claude/commands"

    mkdir -p "$scripts_dir" "$hooks_dir" "$commands_dir"

    log_info "Downloading hook scripts..."

    # Core scripts
    download "${RAW_URL}/.claude/scripts/ensure-local-binary.sh" "${scripts_dir}/ensure-local-binary.sh"
    download "${RAW_URL}/.claude/scripts/run-py.sh" "${scripts_dir}/run-py.sh"
    download "${RAW_URL}/.claude/scripts/startup-hook.py" "${scripts_dir}/startup-hook.py"
    download "${RAW_URL}/.claude/scripts/precompact-beads-hook.py" "${scripts_dir}/precompact-beads-hook.py"
    download "${RAW_URL}/.claude/scripts/quality-check.sh" "${scripts_dir}/quality-check.sh"
    download "${RAW_URL}/.claude/scripts/autonomous-stop-hook.py" "${scripts_dir}/autonomous-stop-hook.py"
    download "${RAW_URL}/.claude/scripts/code-review-hook.py" "${scripts_dir}/code-review-hook.py"

    # Hook wrappers
    download "${RAW_URL}/.claude/scripts/hooks/stop.sh" "${hooks_dir}/stop.sh"
    download "${RAW_URL}/.claude/scripts/hooks/pre-tool-use-no-verify.sh" "${hooks_dir}/pre-tool-use-no-verify.sh"
    download "${RAW_URL}/.claude/scripts/hooks/pre-tool-use-code-review.sh" "${hooks_dir}/pre-tool-use-code-review.sh"

    # Make scripts executable
    chmod +x "${scripts_dir}"/*.sh "${scripts_dir}"/*.py 2>/dev/null || true
    chmod +x "${hooks_dir}"/*.sh

    # Commands (slash commands)
    download "${RAW_URL}/.claude/commands/autonomous-mode.md" "${commands_dir}/autonomous-mode.md"
    download "${RAW_URL}/.claude/commands/cancel-autonomous.md" "${commands_dir}/cancel-autonomous.md"
    download "${RAW_URL}/.claude/commands/quality-check.md" "${commands_dir}/quality-check.md"
    download "${RAW_URL}/.claude/commands/checkpoint.md" "${commands_dir}/checkpoint.md"
    download "${RAW_URL}/.claude/commands/self-review.md" "${commands_dir}/self-review.md"
    download "${RAW_URL}/.claude/commands/ideate.md" "${commands_dir}/ideate.md"

    log_info "Scripts installed to ${scripts_dir}"
}

# Install settings.json (merging with existing if present)
install_settings() {
    local target_dir="$1"
    local settings_file="${target_dir}/.claude/settings.json"

    if [[ -f "$settings_file" ]]; then
        log_warn "settings.json already exists at ${settings_file}"
        log_warn "Please manually merge the hooks from:"
        log_warn "  ${RAW_URL}/.claude/settings.json"
        return
    fi

    log_info "Downloading settings.json..."
    download "${RAW_URL}/.claude/settings.json" "$settings_file"
    log_info "Settings installed to ${settings_file}"
}

# Main installation
main() {
    local target_dir="."

    # Parse arguments
    while [[ $# -gt 0 ]]; do
        case "$1" in
            --target-dir)
                target_dir="$2"
                shift 2
                ;;
            --help|-h)
                echo "Usage: $0 [--target-dir /path/to/project]"
                echo ""
                echo "Install claude-reliability hooks into a Claude Code project."
                echo ""
                echo "Options:"
                echo "  --target-dir DIR    Install to specified directory (default: current directory)"
                echo "  --help, -h          Show this help message"
                exit 0
                ;;
            *)
                log_error "Unknown option: $1"
                exit 1
                ;;
        esac
    done

    # Resolve to absolute path
    target_dir="$(cd "$target_dir" && pwd)"

    log_info "Installing claude-reliability to ${target_dir}"

    # Detect platform
    local artifact_name
    artifact_name=$(detect_platform)
    log_info "Detected platform: ${artifact_name}"

    # Get latest version
    local version
    version=$(get_latest_version)
    log_info "Latest version: ${version}"

    # Create .claude directory
    mkdir -p "${target_dir}/.claude"

    # Install components
    install_binary "$target_dir" "$artifact_name" "$version"
    install_scripts "$target_dir"
    install_settings "$target_dir"

    echo ""
    log_info "Installation complete!"
    echo ""
    echo "Next steps:"
    echo "  1. Review .claude/settings.json to ensure hooks are configured correctly"
    echo "  2. Add .claude/bin/ to .gitignore (binaries should not be committed)"
    echo "  3. Commit the .claude/ directory (excluding binaries)"
    echo ""
    echo "The plugin provides:"
    echo "  - Pre-commit code review hook"
    echo "  - No-verify flag detection"
    echo "  - Stop hook with quality checks"
    echo "  - Autonomous mode commands"
    echo ""
}

main "$@"
