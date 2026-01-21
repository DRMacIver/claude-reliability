# List available commands
# Docker image name
# Build the project
# Check next release version
# Smoke test - verify Rust tools are installed

default:
	@just --list

DOCKER_IMAGE := "claude-reliability-dev"

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

build:
	cargo build

build-release:
	cargo build --release

check: lint test-cov snapshot-tests

check-bin-size:
	#!/usr/bin/env bash
	set -euo pipefail
	MAX_LINES=50
	FAILED=0
	for file in src/main.rs src/bin/*.rs; do
		[ -f "$file" ] || continue
		lines=$(wc -l < "$file")
		if [ "$lines" -gt "$MAX_LINES" ]; then
			echo "ERROR: $file has $lines lines (max $MAX_LINES)"
			echo "Binary entry points must be thin wrappers around library code."
			echo "Move logic to src/lib.rs and call it from main()."
			FAILED=1
		fi
	done
	if [ "$FAILED" -eq 1 ]; then
		exit 1
	fi
	echo "All binary entry points are thin wrappers (≤$MAX_LINES lines)"

clean:
	cargo clean

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

doc:
	cargo doc --no-deps --open

format:
	cargo fmt

install-tools:
	rustup component add clippy rustfmt llvm-tools-preview
	cargo install cargo-llvm-cov

lint: check-bin-size
	cargo clippy --all-targets --all-features -- -D warnings
	cargo fmt --check

release:
	#!/usr/bin/env bash
	set -euo pipefail
	# Update version in Cargo.toml
	python3 scripts/release.py
	# Get the version that was set
	VERSION=$(python3 scripts/release.py --version-only)
	# Commit version update
	git add Cargo.toml
	git commit -m "Release $VERSION"
	# Create and push tag - this triggers the GitHub Actions release workflow
	git tag "v$VERSION"
	git push origin main "v$VERSION"
	echo ""
	echo "Release v$VERSION initiated!"
	echo "GitHub Actions will now build binaries for all platforms."
	echo "Monitor progress at: https://github.com/$(git remote get-url origin | sed 's/.*github.com[:/]//;s/.git$//')/actions"

release-preview:
	python3 scripts/release.py --dry-run

release-version:
	python3 scripts/release.py --version-only

run *ARGS:
	cargo run --features cli -- {{ARGS}}

# Snapshot test commands
# Run snapshot tests in replay mode (default)
snapshot-tests *ARGS:
	cd snapshot-tests && uv run python -m snapshot_tests.run_snapshots {{ARGS}}

# Run snapshot tests with verbose output
snapshot-tests-verbose *ARGS:
	cd snapshot-tests && uv run python -m snapshot_tests.run_snapshots --verbose {{ARGS}}

# Record new snapshot transcripts (requires Claude Code)
snapshot-tests-record *ARGS:
	cd snapshot-tests && uv run python -m snapshot_tests.run_snapshots --mode=record {{ARGS}}

# Compile transcript.jsonl to human-readable markdown
compile-transcript FILE:
	cd snapshot-tests && uv run python -m snapshot_tests.compile_transcript {{FILE}}

smoke-test:
	#!/usr/bin/env bash
	set -e
	echo "Checking essential tools..."
	command -v just >/dev/null || { echo "ERROR: just not installed"; exit 1; }
	command -v git >/dev/null || { echo "ERROR: git not installed"; exit 1; }
	command -v claude >/dev/null || { echo "ERROR: claude not installed"; exit 1; }
	# Run language-specific smoke tests if they exist
	for recipe in $(just --list --unsorted 2>/dev/null | grep '^smoke-test-' | awk '{print $1}'); do
		echo "Running $recipe..."
		just "$recipe"
	done
	echo "All essential tools present"

smoke-test-rust:
	#!/usr/bin/env bash
	set -e
	command -v cargo >/dev/null || { echo "ERROR: cargo not installed"; exit 1; }
	command -v rustc >/dev/null || { echo "ERROR: rustc not installed"; exit 1; }
	rustc --version
	echo "Rust tools present"

test *ARGS:
	cargo test {{ARGS}}

test-cov:
	./scripts/check-coverage.py

update-my-hooks:
	#!/usr/bin/env bash
	set -euo pipefail
	echo "Building claude-reliability binary..."
	cargo build --release --features cli
	mkdir -p .claude/bin
	cp target/release/claude-reliability .claude/bin/
	chmod +x .claude/bin/claude-reliability
	echo "Installed to .claude/bin/claude-reliability"
	echo ""
	echo "Binary version:"
	.claude/bin/claude-reliability version

validate-devcontainer:
	@test -f .devcontainer/devcontainer.json && test -f .devcontainer/Dockerfile && echo "Devcontainer configuration valid"
