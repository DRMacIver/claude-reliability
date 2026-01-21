#!/bin/bash
# Create beads issues for all tests that don't have them yet
# This script is faster than the Python version because it batches operations

set -uo pipefail

# Get all test names
ALL_TESTS=$(grep -rho 'fn test_\w*' src/*.rs src/**/*.rs 2>/dev/null | sed 's/fn //' | sort -u)

# Get existing issue test names (use --no-daemon to avoid daemon startup delay)
EXISTING=$(bd --no-daemon list --status=open --limit 0 2>/dev/null | grep "Audit test isolation:" | sed 's/.*Audit test isolation: //' | sort -u)

# Find missing tests
MISSING=$(comm -23 <(echo "$ALL_TESTS") <(echo "$EXISTING"))

COUNT=$(echo "$MISSING" | grep -c . || echo 0)
echo "Creating issues for $COUNT tests..."

CREATED=0
for TEST in $MISSING; do
    if [ -z "$TEST" ]; then continue; fi

    # Find the file containing this test
    FILE=$(grep -rl "fn $TEST" src/*.rs src/**/*.rs 2>/dev/null | head -1 || echo "unknown")

    TITLE="Audit test isolation: $TEST"
    BODY="Investigate test \`$TEST\` in \`$FILE\` for global state dependencies.

## Investigation Checklist
- [ ] Does this test depend on current working directory?
- [ ] Does this test depend on environment variables?
- [ ] Does this test use file paths outside of TempDir?
- [ ] Can this test fail if run in parallel with other tests?

## Action Required
If test depends on global state:
1. Fix to be isolated (use TempDir, mock env vars), OR
2. Add \`#[serial_test::serial]\` if unfixable

If already isolated: Close as no action needed."

    if bd --no-daemon create --title "$TITLE" --type task --priority 4 <<< "$BODY" >/dev/null 2>&1; then
        CREATED=$((CREATED + 1))
        echo "  [$CREATED] $TEST"
    else
        echo "  FAILED: $TEST"
    fi
done

echo ""
echo "Created $CREATED issues"
