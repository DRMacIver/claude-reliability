#!/bin/bash
set -e

# Copy Claude config into container
if [ -d /mnt/host-claude ]; then
    mkdir -p ~/.claude
    rsync -av /mnt/host-claude/ ~/.claude/
fi

# Copy SSH keys with correct permissions
if [ -d /mnt/host-ssh ]; then
    mkdir -p ~/.ssh
    chmod 700 ~/.ssh
    rsync -av /mnt/host-ssh/ ~/.ssh/
    chmod 600 ~/.ssh/id_* 2>/dev/null || true
    chmod 644 ~/.ssh/*.pub 2>/dev/null || true
fi

# Update beads to latest (Claude Code auto-updates via native installer)
sudo npm install -g @beads/bd || true

# Configure beads
if [ ! -d .beads ]; then
    # New project - initialize beads
    bd init || true
else
    # Cloned project - ensure beads is properly configured
    # Fix repo fingerprint if needed (common after cloning)
    if bd doctor 2>&1 | grep -q "Repo Fingerprint.*different repository"; then
        echo "y" | bd migrate --update-repo-id || true
    fi
fi

# Make all git hooks executable
if [ -d .githooks ]; then
    chmod +x .githooks/* 2>/dev/null || true
fi

echo "Development environment ready!"

# Install Rust toolchain components and tools
if command -v rustup &> /dev/null; then
    rustup component add clippy rustfmt llvm-tools-preview
    cargo install cargo-llvm-cov
fi
