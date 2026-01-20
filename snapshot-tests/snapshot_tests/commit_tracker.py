"""Commit SHA placeholders for snapshot tests.

Git commit SHAs differ between recording and replay. This module provides
a simple placeholder system where `<<commit:ORIGINAL_SHA>>` matches any
valid commit SHA prefix (7+ hex characters).

Usage:
    # In expected output from recording:
    "007c8c1 Initial commit"

    # Gets normalized to:
    "<<commit:007c8c1>> Initial commit"

    # During replay, matches:
    "f9e3260 Initial commit"

    # Because <<commit:...>> matches any 7+ hex string
"""

from __future__ import annotations

import re


# Pattern for commit placeholder: <<commit:SHA>>
COMMIT_PLACEHOLDER = re.compile(r"<<commit:([0-9a-f]{7,40})>>")

# Pattern for a git SHA (7-40 hex chars)
SHA_PATTERN = re.compile(r"[0-9a-f]{7,40}")


def normalize_expected(text: str) -> str:
    """Replace git SHAs in expected output with commit placeholders.

    Recognizes SHAs in common git output formats:
    - git log: "007c8c1 Initial commit"
    - git commit: "[master 99dc541] Add function"
    - git diff: "index cd53e7d..086743a"

    Args:
        text: Expected output from recorded transcript

    Returns:
        Text with SHAs replaced by <<commit:SHA>> placeholders
    """
    result = text

    # Find all potential SHAs (7+ hex chars at word boundaries)
    # We need to be careful not to match other hex strings

    # Pattern 1: git log --oneline format: "SHA message"
    result = re.sub(
        r"^([0-9a-f]{7,40})(\s+)",
        r"<<commit:\1>>\2",
        result,
        flags=re.MULTILINE
    )

    # Pattern 2: git commit output: "[branch SHA] message"
    result = re.sub(
        r"\[(\w+)\s+([0-9a-f]{7,40})\]",
        r"[\1 <<commit:\2>>]",
        result
    )

    # Pattern 3: git diff index line: "index SHA..SHA"
    result = re.sub(
        r"index\s+([0-9a-f]{7,40})\.\.([0-9a-f]{7,40})",
        r"index <<commit:\1>>..<<commit:\2>>",
        result
    )

    return result


def normalize_actual(text: str) -> str:
    """Replace git SHAs in actual output with a generic placeholder.

    This uses the same placeholder format so comparison works.
    The actual SHA value doesn't matter for matching - any valid
    SHA prefix will match any <<commit:...>> placeholder.

    Args:
        text: Actual output from replay

    Returns:
        Text with SHAs replaced by <<commit:SHA>> placeholders
    """
    # Use the same normalization - the key insight is that
    # <<commit:X>> == <<commit:Y>> for comparison purposes
    return normalize_expected(text)


def placeholders_match(expected: str, actual: str) -> bool:
    """Check if expected and actual match, treating commit placeholders as equivalent.

    Two commit placeholders match if they're both valid SHA prefixes,
    regardless of the actual SHA value.

    Args:
        expected: Normalized expected text with <<commit:SHA>> placeholders
        actual: Normalized actual text with <<commit:SHA>> placeholders

    Returns:
        True if the texts match (with placeholder equivalence)
    """
    # Replace all commit placeholders with a canonical form for comparison
    canonical_expected = COMMIT_PLACEHOLDER.sub("<<commit>>", expected)
    canonical_actual = COMMIT_PLACEHOLDER.sub("<<commit>>", actual)

    return canonical_expected == canonical_actual


def normalize_git_output(expected: str, actual: str) -> tuple[str, str]:
    """Normalize both expected and actual for comparison.

    Args:
        expected: Expected output from recorded transcript
        actual: Actual output from replay

    Returns:
        Tuple of (normalized_expected, normalized_actual) ready for comparison
    """
    norm_expected = normalize_expected(expected)
    norm_actual = normalize_actual(actual)

    # Canonicalize placeholders for comparison
    canon_expected = COMMIT_PLACEHOLDER.sub("<<commit>>", norm_expected)
    canon_actual = COMMIT_PLACEHOLDER.sub("<<commit>>", norm_actual)

    return canon_expected, canon_actual
