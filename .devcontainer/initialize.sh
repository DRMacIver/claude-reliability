#!/bin/bash
# Runs on the HOST before the container starts
# Generates credentials that get mounted into the container

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
CREDS_DIR="$PROJECT_DIR/.devcontainer/.credentials"
SSH_DIR="$PROJECT_DIR/.devcontainer/.ssh"

# Create directories
mkdir -p "$CREDS_DIR"
mkdir -p "$SSH_DIR"

# Ensure scripts are executable
chmod +x "$SCRIPT_DIR/entrypoint.sh" 2>/dev/null || true
chmod +x "$SCRIPT_DIR/post-create.sh" 2>/dev/null || true

# Ensure Claude Code scripts are executable
chmod +x "$PROJECT_DIR/.claude/scripts/"*.sh 2>/dev/null || true

# Set up SSH deploy key for git push (using host's gh CLI with full permissions)
if ! command -v gh &> /dev/null; then
    echo "SSH: gh CLI not found, skipping deploy key setup"
elif ! gh auth status &> /dev/null; then
    echo "SSH: gh not authenticated, skipping deploy key setup"
else
    SSH_KEY="$SSH_DIR/devcontainer"

    # Generate key if not exists
    if [ ! -f "$SSH_KEY" ]; then
        ssh-keygen -t ed25519 -f "$SSH_KEY" -N "" -C "devcontainer-$(basename "$PROJECT_DIR")" > /dev/null
        echo "SSH: generated new key pair"
    else
        echo "SSH: using existing key pair"
    fi

    # Get repo info using gh
    cd "$PROJECT_DIR"
    REPO_INFO=$(gh repo view --json owner,name 2>/dev/null || echo "")
    if [ -n "$REPO_INFO" ]; then
        OWNER=$(echo "$REPO_INFO" | jq -r '.owner.login')
        REPO=$(echo "$REPO_INFO" | jq -r '.name')
        echo "SSH: setting up deploy key for $OWNER/$REPO"

        KEY_TITLE="Devcontainer (auto-generated)"
        PUB_KEY=$(cat "$SSH_KEY.pub")

        # Check existing deploy keys
        EXISTING=$(gh api "repos/$OWNER/$REPO/keys" 2>/dev/null || echo "[]")

        # Find our key by content (first two parts of public key)
        OUR_KEY_CONTENT=$(echo "$PUB_KEY" | awk '{print $1" "$2}')
        FOUND_KEY=$(echo "$EXISTING" | jq -r --arg key "$OUR_KEY_CONTENT" '.[] | select(.key == $key)')

        if [ -n "$FOUND_KEY" ]; then
            KEY_ID=$(echo "$FOUND_KEY" | jq -r '.id')
            IS_READ_ONLY=$(echo "$FOUND_KEY" | jq -r '.read_only')

            if [ "$IS_READ_ONLY" = "true" ]; then
                # Delete read-only key and re-add with write access
                gh api -X DELETE "repos/$OWNER/$REPO/keys/$KEY_ID" > /dev/null
                gh api "repos/$OWNER/$REPO/keys" -f title="$KEY_TITLE" -f key="$PUB_KEY" -F read_only=false > /dev/null
                echo "SSH: replaced deploy key (now with write access)"
            else
                echo "SSH: deploy key already has write access"
            fi
        else
            # Add new key with write access
            if gh api "repos/$OWNER/$REPO/keys" -f title="$KEY_TITLE" -f key="$PUB_KEY" -F read_only=false > /dev/null 2>&1; then
                echo "SSH: added deploy key with write access"
            else
                echo "SSH: could not add deploy key (may already exist)"
            fi
        fi

        # Set remote URL to SSH format
        git remote set-url origin "git@github.com:$OWNER/$REPO.git"
        echo "SSH: set remote to git@github.com:$OWNER/$REPO.git"
    else
        echo "SSH: could not get repo info (not a GitHub repo?)"
    fi
fi

echo "Host initialization complete."
