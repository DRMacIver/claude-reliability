#!/bin/bash
# Container entrypoint - runs setup if needed, then executes command
set -e

# Fix home directory ownership if needed (handles volume mounts from old vscode user)
# The dev user has passwordless sudo, so this works even when running as dev
if [ ! -w "$HOME" ] 2>/dev/null; then
    echo "Fixing home directory permissions..."
    sudo mkdir -p "$HOME"
    sudo chown -R dev:dev "$HOME"
fi

SETUP_MARKER="$HOME/.container-setup-done"

# Set up environment
# Include uv tools, Claude, and the project's virtualenv (if UV_PROJECT_ENVIRONMENT is set)
export PATH="/home/dev/.local/bin:/home/dev/.claude/bin:$PATH"
if [ -n "$UV_PROJECT_ENVIRONMENT" ]; then
    export PATH="$UV_PROJECT_ENVIRONMENT/bin:$PATH"
    # Ensure venv directory parent exists
    mkdir -p "$(dirname "$UV_PROJECT_ENVIRONMENT")"
fi

# Set Claude config directory
export CLAUDE_CONFIG_DIR="$HOME/.claude"

# Install Claude Code if not present
# We install here (not in Dockerfile) because /home/dev is a volume mount
# and would shadow any files installed during build
CLAUDE_INSTALL_MARKER="$HOME/.claude-installed"
if [ ! -f "$CLAUDE_INSTALL_MARKER" ] || [ ! -x "$HOME/.claude/bin/claude" ]; then
    echo "Installing Claude Code..."
    curl -fsSL https://claude.ai/install.sh | bash
    touch "$CLAUDE_INSTALL_MARKER"
    echo "Claude Code installed successfully"
fi

# Copy Claude credentials from host if not present in container
# Claude Code needs:
# 1. OAuth tokens in ~/.claude/.credentials.json
# 2. Config with oauthAccount in ~/.claude/.claude.json (identifies logged-in user)
CLAUDE_DIR_HOME="$HOME/.claude"
CLAUDE_KEYCHAIN_HOST="/mnt/credentials/claude-keychain.json"
CLAUDE_CONFIG_HOST="/mnt/credentials/claude-config.json"
CLAUDE_COPY_MARKER="$HOME/.claude-credentials-copied"

# Check if we need to copy credentials
# We re-copy if: marker missing, credentials missing, OR config missing
NEED_CREDS_COPY="no"
if [ ! -f "$CLAUDE_COPY_MARKER" ]; then
    NEED_CREDS_COPY="yes"
elif [ ! -f "$CLAUDE_DIR_HOME/.credentials.json" ]; then
    NEED_CREDS_COPY="yes"
    rm -f "$CLAUDE_COPY_MARKER"
elif [ ! -f "$CLAUDE_DIR_HOME/.claude.json" ]; then
    NEED_CREDS_COPY="yes"
    rm -f "$CLAUDE_COPY_MARKER"
fi

if [ "$NEED_CREDS_COPY" = "yes" ]; then
    echo "Setting up Claude credentials..."
    mkdir -p "$CLAUDE_DIR_HOME"
    CREDS_OK="yes"

    # Copy OAuth tokens
    if [ -f "$CLAUDE_KEYCHAIN_HOST" ] && [ -s "$CLAUDE_KEYCHAIN_HOST" ]; then
        cp "$CLAUDE_KEYCHAIN_HOST" "$CLAUDE_DIR_HOME/.credentials.json"
        chmod 600 "$CLAUDE_DIR_HOME/.credentials.json"
        echo "  OAuth tokens: copied ($(wc -c < "$CLAUDE_DIR_HOME/.credentials.json") bytes)"
    else
        echo "  WARNING: OAuth tokens not found at $CLAUDE_KEYCHAIN_HOST"
        CREDS_OK="no"
    fi

    # Copy config file (contains oauthAccount)
    if [ -f "$CLAUDE_CONFIG_HOST" ] && [ -s "$CLAUDE_CONFIG_HOST" ]; then
        cp "$CLAUDE_CONFIG_HOST" "$CLAUDE_DIR_HOME/.claude.json"
        chmod 600 "$CLAUDE_DIR_HOME/.claude.json"
        echo "  Config file: copied ($(wc -c < "$CLAUDE_DIR_HOME/.claude.json") bytes)"
    else
        echo "  WARNING: Config file not found at $CLAUDE_CONFIG_HOST"
        CREDS_OK="no"
    fi

    if [ "$CREDS_OK" = "yes" ]; then
        touch "$CLAUDE_COPY_MARKER"
        echo "  Claude credentials setup complete"
    else
        echo "  WARNING: Incomplete credentials - Claude may ask to log in"
        echo "  Run 'claude' on macOS host and log in first"
    fi
fi

# Set up GitHub credentials (if available)
# Uses:
# - SSH deploy keys for git push (permanent, per-project)
# - GitHub App token for API calls like gh CLI (expires but refreshed)
GH_TOKEN_FILE="/mnt/credentials/github_token.json"
if [ -f "$GH_TOKEN_FILE" ] && [ -s "$GH_TOKEN_FILE" ]; then
    mkdir -p ~/.local/bin

    # For gh CLI, create a wrapper script that shadows /usr/bin/gh
    # This reads the token fresh each time (handles refresh on host)
    cat > ~/.local/bin/gh << 'GH_WRAPPER'
#!/bin/bash
# Wrapper that sets GH_TOKEN fresh from file before running gh
TOKEN_FILE="/mnt/credentials/github_token.json"
if [ -f "$TOKEN_FILE" ] && [ -s "$TOKEN_FILE" ]; then
    if ! TOKEN=$(jq -r '.token' "$TOKEN_FILE" 2>&1); then
        echo "WARNING: Failed to parse $TOKEN_FILE: $TOKEN" >&2
        TOKEN=""
    fi
    if [ -n "$TOKEN" ] && [ "$TOKEN" != "null" ]; then
        export GH_TOKEN="$TOKEN"
    else
        echo "WARNING: No valid token found in $TOKEN_FILE" >&2
    fi
elif [ ! -f "$TOKEN_FILE" ]; then
    echo "WARNING: Token file not found: $TOKEN_FILE" >&2
    echo "Run 'just develop' from the host to generate a GitHub token" >&2
fi
exec /usr/bin/gh "$@"
GH_WRAPPER
    chmod +x ~/.local/bin/gh

    # Set up SSH for git push (deploy key and remote URL set up on host by initialize.sh)
    python3 << 'SETUP_SSH'
import shutil
import subprocess
import sys
from pathlib import Path

SSH_HOST_DIR = Path("/mnt/ssh-keys")
SSH_KEY = SSH_HOST_DIR / "devcontainer"
SSH_DIR = Path.home() / ".ssh"

if not SSH_KEY.exists():
    print("SSH: no deploy key found (run 'just develop' from host to set up)")
    sys.exit(0)

# Copy SSH key from host mount to container
SSH_DIR.mkdir(parents=True, exist_ok=True)
SSH_DIR.chmod(0o700)

private_key = SSH_DIR / "devcontainer"
public_key = SSH_DIR / "devcontainer.pub"

shutil.copy(SSH_KEY, private_key)
shutil.copy(SSH_KEY.with_suffix(".pub"), public_key)
private_key.chmod(0o600)
public_key.chmod(0o644)

# Configure SSH to use the key for GitHub
ssh_config = SSH_DIR / "config"
ssh_config.write_text(
    "Host github.com\n"
    "    HostName github.com\n"
    "    User git\n"
    f"    IdentityFile {private_key}\n"
    "    IdentitiesOnly yes\n"
)
ssh_config.chmod(0o600)

# Add github.com to known hosts
known_hosts = SSH_DIR / "known_hosts"
result = subprocess.run(
    ["ssh-keyscan", "github.com"],
    capture_output=True, text=True
)
if result.returncode == 0:
    with open(known_hosts, "a") as f:
        f.write(result.stdout)

# Remote URL was already set to SSH by host initialization
print("SSH: configured for git push")
SETUP_SSH

    echo "GitHub: credentials configured"
fi

# Configure git identity from host environment variables
GIT_CONFIG_MARKER="$HOME/.git-identity-configured"
if [ ! -f "$GIT_CONFIG_MARKER" ]; then
    if [ -n "$GIT_USER_NAME" ]; then
        git config --global user.name "$GIT_USER_NAME"
        echo "Git: user.name set to '$GIT_USER_NAME'"
    fi
    if [ -n "$GIT_USER_EMAIL" ]; then
        git config --global user.email "$GIT_USER_EMAIL"
        echo "Git: user.email set to '$GIT_USER_EMAIL'"
    fi
    if [ -n "$GIT_USER_NAME" ] || [ -n "$GIT_USER_EMAIL" ]; then
        touch "$GIT_CONFIG_MARKER"
    fi
fi

# Configure git hooks path for project (if .githooks/ exists)
PROJECT_DIR="/workspaces/claude-reliability"
if [ -d "$PROJECT_DIR/.githooks" ] && [ -d "$PROJECT_DIR/.git" ]; then
    # Set hooks path at repo level (not global)
    git -C "$PROJECT_DIR" config core.hooksPath .githooks
    echo "Git: hooksPath set to .githooks"
fi

# Set up node_modules in /home/dev for TypeScript projects (similar to Python's UV_PROJECT_ENVIRONMENT)
# This prevents host/container node_modules conflicts
PROJECT_DIR="/workspaces/claude-reliability"
NODE_MODULES_TARGET="$HOME/node_modules/claude-reliability"
if [ -f "$PROJECT_DIR/package.json" ]; then
    # Create target directory in home volume
    mkdir -p "$NODE_MODULES_TARGET"

    # If node_modules doesn't exist or is already a symlink, set up the symlink
    if [ ! -e "$PROJECT_DIR/node_modules" ] || [ -L "$PROJECT_DIR/node_modules" ]; then
        rm -f "$PROJECT_DIR/node_modules" 2>/dev/null || true
        ln -sf "$NODE_MODULES_TARGET" "$PROJECT_DIR/node_modules"
        echo "Node: node_modules linked to $NODE_MODULES_TARGET"
    fi
fi

# Run setup if not done yet (or if post-create.sh changed)
POST_CREATE="/workspaces/claude-reliability/.devcontainer/post-create.sh"
POST_CREATE_HASH=$(sha256sum "$POST_CREATE" 2>/dev/null | cut -d' ' -f1 || echo "none")
CACHED_HASH=""
if [ -f "$SETUP_MARKER" ]; then
    CACHED_HASH=$(cat "$SETUP_MARKER")
fi

if [ "$POST_CREATE_HASH" != "$CACHED_HASH" ]; then
    echo "Running container setup..."
    cd /workspaces/claude-reliability
    bash "$POST_CREATE"

    # Mark setup as done
    echo "$POST_CREATE_HASH" > "$SETUP_MARKER"
    echo "Container setup complete!"
fi

# Execute the provided command
exec "$@"
