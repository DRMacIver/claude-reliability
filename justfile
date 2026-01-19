# List available commands
default:
	@just --list

# Docker image name
DOCKER_IMAGE := "claude-reliability-dev"

# Validate devcontainer configuration files exist
validate-devcontainer:
	@test -f .devcontainer/devcontainer.json && test -f .devcontainer/Dockerfile && echo "Devcontainer configuration valid"

# Build Docker image if Dockerfile changed
_docker-build:
	#!/usr/bin/env bash
	set -e
	HASH=$(cat .devcontainer/Dockerfile | sha256sum | cut -d' ' -f1)
	SENTINEL=".devcontainer/.docker-build-hash"
	CACHED_HASH=""
	if [ -f "$SENTINEL" ]; then
		CACHED_HASH=$(cat "$SENTINEL")
	fi
	if [ "$HASH" != "$CACHED_HASH" ]; then
		echo "Dockerfile changed, rebuilding image..."
		docker build -t {{DOCKER_IMAGE}} -f .devcontainer/Dockerfile .
		echo "$HASH" > "$SENTINEL"
	fi

# Start development container and run claude (or custom command if args provided)
develop *ARGS:
	#!/usr/bin/env bash
	set -e

	# Validate devcontainer configuration exists
	if [ ! -f .devcontainer/devcontainer.json ]; then
		echo "Error: .devcontainer/devcontainer.json not found"
		exit 1
	fi
	if [ ! -f .devcontainer/Dockerfile ]; then
		echo "Error: .devcontainer/Dockerfile not found"
		exit 1
	fi
	# Validate JSON syntax
	python3 -c "import json; json.load(open('.devcontainer/devcontainer.json'))"

	# Build image if needed
	just _docker-build

	# Run host initialization
	bash .devcontainer/initialize.sh

	# Generate GitHub token if GitHub App is configured
	bash .devcontainer/scripts/generate-github-token.sh

	# Extract Claude credentials from macOS
	# Claude Code needs two things:
	# 1. OAuth tokens from Keychain -> .credentials.json
	# 2. Config file with oauthAccount -> .claude.json (tells Claude who is logged in)
	CLAUDE_CREDS_DIR="$(pwd)/.devcontainer/.credentials"
	mkdir -p "$CLAUDE_CREDS_DIR"

	if command -v security &> /dev/null; then
		echo "Extracting Claude credentials from macOS..."

		# Extract OAuth tokens from Keychain
		CLAUDE_KEYCHAIN_FILE="$CLAUDE_CREDS_DIR/claude-keychain.json"
		security find-generic-password -s "Claude Code-credentials" -w 2>/dev/null > "$CLAUDE_KEYCHAIN_FILE" || true
		if [ -s "$CLAUDE_KEYCHAIN_FILE" ]; then
			echo "  OAuth tokens: $(wc -c < "$CLAUDE_KEYCHAIN_FILE") bytes"
		else
			echo "  WARNING: No OAuth tokens in Keychain"
			echo "  Run 'claude' on macOS and log in first"
			rm -f "$CLAUDE_KEYCHAIN_FILE"
		fi

		# Copy Claude config file (contains oauthAccount which identifies logged-in user)
		CLAUDE_CONFIG_FILE="$CLAUDE_CREDS_DIR/claude-config.json"
		if [ -f "$HOME/.claude/.claude.json" ]; then
			cp "$HOME/.claude/.claude.json" "$CLAUDE_CONFIG_FILE"
			echo "  Config file: copied from ~/.claude/.claude.json"
		elif [ -f "$HOME/.claude.json" ]; then
			cp "$HOME/.claude.json" "$CLAUDE_CONFIG_FILE"
			echo "  Config file: copied from ~/.claude.json"
		else
			echo "  WARNING: No Claude config file found"
			echo "  Run 'claude' on macOS and complete login first"
		fi
	else
		echo "Note: Not running on macOS, skipping credential extraction"
	fi

	# Extract git identity from host for use in container
	GIT_USER_NAME=$(git config --global user.name 2>/dev/null || echo "")
	GIT_USER_EMAIL=$(git config --global user.email 2>/dev/null || echo "")

	# Detect terminal background color mode
	THEME="light-ansi"
	if [ -n "$COLORFGBG" ]; then
		BG=$(echo "$COLORFGBG" | cut -d';' -f2)
		if [ "$BG" -lt 7 ] 2>/dev/null; then
			THEME="dark-ansi"
		fi
	elif [ "$TERM_BACKGROUND" = "dark" ]; then
		THEME="dark-ansi"
	fi

	# Determine command to run
	if [ -z "{{ARGS}}" ]; then
		SETTINGS="{\"theme\":\"$THEME\"}"
		DOCKER_CMD="claude --dangerously-skip-permissions --settings '$SETTINGS'"
	else
		DOCKER_CMD="{{ARGS}}"
	fi

	# Detect if we have a TTY for interactive mode
	if [ -t 0 ]; then
		INTERACTIVE_FLAGS="-it"
	else
		INTERACTIVE_FLAGS="-t"
	fi

	# Run container with all necessary mounts
	# UV_PROJECT_ENVIRONMENT puts virtualenv in /home/dev (a volume) to avoid host/container conflicts
	# This also allows hardlinks to work since venv and uv cache are on the same filesystem
	docker run $INTERACTIVE_FLAGS --rm \
		-v "$(pwd):/workspaces/claude-reliability" \
		-v "$(pwd)/.devcontainer/.credentials:/mnt/credentials:ro" \
		-v "$(pwd)/.devcontainer/.ssh:/mnt/ssh-keys" \
		-v "claude-reliability-home:/home/dev" \
		-v "claude-reliability-.cache:/workspaces/claude-reliability/.cache" \
		-e ANTHROPIC_API_KEY= \
		-e UV_PROJECT_ENVIRONMENT=/home/dev/venvs/claude-reliability \
		-e GIT_USER_NAME="$GIT_USER_NAME" \
		-e GIT_USER_EMAIL="$GIT_USER_EMAIL" \
		-w /workspaces/claude-reliability \
		--user dev \
		--entrypoint /workspaces/claude-reliability/.devcontainer/entrypoint.sh \
		{{DOCKER_IMAGE}} \
		bash -c "$DOCKER_CMD"

# Build the project
build:
	cargo build

# Build release version
build-release:
	cargo build --release

# Run tests (single-threaded due to env var tests in no_verify)
test *ARGS:
	cargo test -- --test-threads=1 {{ARGS}}

# Run tests with coverage (requires cargo-llvm-cov)
test-cov:
	cargo llvm-cov --all-features --fail-under-lines 100

# Run linter (clippy)
lint:
	cargo clippy --all-targets --all-features -- -D warnings
	cargo fmt --check

# Format code
format:
	cargo fmt

# Run all checks
check: lint test

# Clean build artifacts
clean:
	cargo clean

# Install development tools
install-tools:
	rustup component add clippy rustfmt llvm-tools-preview
	cargo install cargo-llvm-cov

# Generate documentation
doc:
	cargo doc --no-deps --open

# Run the CLI
run *ARGS:
	cargo run -- {{ARGS}}
