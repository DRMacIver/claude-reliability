#!/bin/bash
set -e

echo "Running quality checks..."

# Base checks - language capabilities will add specific ones
if command -v just &> /dev/null && [ -f justfile ]; then
    just check
else
    echo "No quality check command configured"
    echo "Add a 'just check' recipe to your justfile"
fi

echo "Quality checks complete!"
