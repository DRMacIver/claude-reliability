#!/usr/bin/env bash
# Snapshot test runner for claude-reliability hooks
#
# Usage:
#   ./run-snapshots.sh [options] [test_name...]
#
# Options:
#   --update    Update snapshots instead of comparing
#   --create    Create new snapshots for tests without transcripts
#   --model     Model to use (default: haiku)
#   --verbose   Show detailed output
#
# Each test is a directory under snapshot-tests/ containing:
#   - story.md: Description of the test scenario
#   - setup.sh: Script to set up the test environment (run in temp git repo)
#   - transcript.md: The expected/actual transcript output (generated)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"

# Default options
UPDATE_MODE=false
CREATE_MODE=false
MODEL="haiku"
VERBOSE=false
SELECTED_TESTS=()

# Parse arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        --update)
            UPDATE_MODE=true
            shift
            ;;
        --create)
            CREATE_MODE=true
            shift
            ;;
        --model)
            MODEL="$2"
            shift 2
            ;;
        --verbose)
            VERBOSE=true
            shift
            ;;
        *)
            SELECTED_TESTS+=("$1")
            shift
            ;;
    esac
done

# Find test directories
find_tests() {
    for dir in "$SCRIPT_DIR"/*/; do
        if [[ -f "${dir}story.md" ]] && [[ -f "${dir}setup.sh" ]]; then
            basename "$dir"
        fi
    done
}

# Run a single test
run_test() {
    local test_name="$1"
    local test_dir="$SCRIPT_DIR/$test_name"
    local transcript_file="$test_dir/transcript.md"
    local temp_dir=""

    echo "Running test: $test_name"

    # Check if test exists
    if [[ ! -f "$test_dir/story.md" ]]; then
        echo "  ERROR: Missing story.md"
        return 1
    fi
    if [[ ! -f "$test_dir/setup.sh" ]]; then
        echo "  ERROR: Missing setup.sh"
        return 1
    fi

    # Check if transcript exists (unless creating)
    if [[ ! -f "$transcript_file" ]] && [[ "$CREATE_MODE" != "true" ]] && [[ "$UPDATE_MODE" != "true" ]]; then
        echo "  SKIP: No transcript.md (use --create to generate)"
        return 0
    fi

    # Create temp directory for test
    temp_dir=$(mktemp -d)

    # Subshell to contain directory changes
    (
        cd "$temp_dir"

        # Initialize git repo
        git init --quiet
        git config user.email "test@example.com"
        git config user.name "Test User"

        # Copy project files needed for hooks
        cp -r "$PROJECT_DIR/target" . 2>/dev/null || true
        mkdir -p .claude/scripts/hooks

        # Run setup script
        if [[ "$VERBOSE" == "true" ]]; then
            echo "  Running setup.sh..."
        fi
        bash "$test_dir/setup.sh"

        # Create initial commit if not already done
        git add -A 2>/dev/null || true
        git commit -m "Initial setup" --allow-empty --quiet 2>/dev/null || true

        # Read the prompt from story.md (first code block)
        local prompt
        prompt=$(sed -n '/```/,/```/p' "$test_dir/story.md" | sed '1d;$d' | head -20)

        if [[ -z "$prompt" ]]; then
            # If no code block, use the whole file as prompt
            prompt=$(cat "$test_dir/story.md")
        fi

        # Run Claude Code with the test
        local output_file="output.md"
        if [[ "$VERBOSE" == "true" ]]; then
            echo "  Running Claude Code..."
        fi

        # TODO: Actually run Claude Code here when infrastructure is ready
        # For now, create a placeholder transcript
        cat > "$output_file" << TRANSCRIPT
# Test: $test_name

## Prompt
\`\`\`
$prompt
\`\`\`

## Transcript

*Transcript will be generated when Claude Code integration is complete.*

TRANSCRIPT

        # Copy output to transcript location
        cp "$output_file" "$transcript_file.new"
    )

    # Compare or update transcript
    if [[ "$UPDATE_MODE" == "true" ]] || [[ "$CREATE_MODE" == "true" ]] || [[ ! -f "$transcript_file" ]]; then
        mv "$transcript_file.new" "$transcript_file"
        echo "  UPDATED: transcript.md"
    else
        if diff -q "$transcript_file" "$transcript_file.new" > /dev/null 2>&1; then
            rm "$transcript_file.new"
            echo "  PASS"
        else
            echo "  FAIL: Transcript differs"
            if [[ "$VERBOSE" == "true" ]]; then
                diff "$transcript_file" "$transcript_file.new" || true
            fi
            rm "$transcript_file.new"
            rm -rf "$temp_dir"
            return 1
        fi
    fi

    # Cleanup
    rm -rf "$temp_dir"
    return 0
}

# Main
main() {
    local tests=()
    local failed=0
    local passed=0
    local skipped=0

    # Get list of tests to run
    if [[ ${#SELECTED_TESTS[@]} -gt 0 ]]; then
        tests=("${SELECTED_TESTS[@]}")
    else
        mapfile -t tests < <(find_tests)
    fi

    if [[ ${#tests[@]} -eq 0 ]]; then
        echo "No tests found"
        exit 0
    fi

    echo "Running ${#tests[@]} snapshot tests..."
    echo

    for test in "${tests[@]}"; do
        if run_test "$test"; then
            ((passed++)) || true
        else
            ((failed++)) || true
        fi
    done

    echo
    echo "Results: $passed passed, $failed failed, $skipped skipped"

    if [[ $failed -gt 0 ]]; then
        exit 1
    fi
}

main
