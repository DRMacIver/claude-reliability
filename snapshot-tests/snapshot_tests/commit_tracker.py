"""Commit SHA placeholders for snapshot tests.

Git commit SHAs differ between recording and replay. This module tracks
commits by order of first appearance and uses numbered placeholders like
<<commit 1>>, <<commit 2>> for consistent matching.

Usage:
    tracker = CommitTracker()

    # First commit seen gets <<commit 1>>
    text1 = tracker.normalize("007c8c1 Initial commit")
    # -> "<<commit 1>> Initial commit"

    # Same commit reused keeps same number
    text2 = tracker.normalize("[master 007c8c1] msg")
    # -> "[master <<commit 1>>] msg"

    # New commit gets next number
    text3 = tracker.normalize("99dc541 Add feature")
    # -> "<<commit 2>> Add feature"
"""

from __future__ import annotations

import re
from dataclasses import dataclass, field


@dataclass
class CommitTracker:
    """Tracks commit SHAs and assigns numbered placeholders.

    Commits are numbered by order of first appearance. The same SHA
    always gets the same placeholder number.
    """

    # Map: SHA prefix (7 chars) -> commit number (1-based)
    sha_to_number: dict[str, int] = field(default_factory=dict)
    # Next commit number to assign
    next_number: int = 1

    def get_placeholder(self, sha: str) -> str:
        """Get or create a placeholder for a SHA.

        Args:
            sha: A git SHA (7-40 hex chars)

        Returns:
            Placeholder like "<<commit 1>>"
        """
        # Normalize to 7-char prefix
        prefix = sha[:7].lower()

        if prefix not in self.sha_to_number:
            self.sha_to_number[prefix] = self.next_number
            self.next_number += 1

        return f"<<commit {self.sha_to_number[prefix]}>>"

    def normalize(self, text: str) -> str:
        """Replace git SHAs with numbered placeholders.

        Recognizes SHAs in common git output formats:
        - git log: "007c8c1 Initial commit"
        - git commit: "[master 99dc541] Add function"
        - git diff: "index cd53e7d..086743a"

        Args:
            text: Text that may contain git SHAs

        Returns:
            Text with SHAs replaced by <<commit N>> placeholders
        """
        result = text

        # Pattern 1: git log --oneline format: "SHA message" at start of line
        def replace_log(m: re.Match) -> str:
            sha = m.group(1)
            space = m.group(2)
            return self.get_placeholder(sha) + space

        result = re.sub(
            r"^([0-9a-f]{7,40})(\s+)",
            replace_log,
            result,
            flags=re.MULTILINE
        )

        # Pattern 2: git commit output: "[branch SHA] message"
        def replace_commit(m: re.Match) -> str:
            branch = m.group(1)
            sha = m.group(2)
            return f"[{branch} {self.get_placeholder(sha)}]"

        result = re.sub(
            r"\[(\w+)\s+([0-9a-f]{7,40})\]",
            replace_commit,
            result
        )

        # Pattern 3: git diff index line: "index SHA..SHA"
        def replace_diff_index(m: re.Match) -> str:
            sha1 = m.group(1)
            sha2 = m.group(2)
            return f"index {self.get_placeholder(sha1)}..{self.get_placeholder(sha2)}"

        result = re.sub(
            r"index\s+([0-9a-f]{7,40})\.\.([0-9a-f]{7,40})",
            replace_diff_index,
            result
        )

        return result


# Pattern to match our commit placeholders
COMMIT_PLACEHOLDER_PATTERN = re.compile(r"<<commit (\d+)>>")


def normalize_for_comparison(expected: str, actual: str) -> tuple[str, str]:
    """Normalize both expected and actual for comparison.

    Uses separate trackers so that commit numbers are assigned independently,
    then compares by structure (<<commit N>> matches <<commit M>> at same position).

    Args:
        expected: Expected output (may already have placeholders or raw SHAs)
        actual: Actual output from replay

    Returns:
        Tuple of (normalized_expected, normalized_actual) ready for comparison
    """
    # Normalize expected (it may already have placeholders from transcript.md)
    expected_tracker = CommitTracker()
    norm_expected = expected_tracker.normalize(expected)

    # Normalize actual
    actual_tracker = CommitTracker()
    norm_actual = actual_tracker.normalize(actual)

    # For comparison, replace all <<commit N>> with just <<commit>>
    # This allows matching regardless of the specific number
    canon_expected = COMMIT_PLACEHOLDER_PATTERN.sub("<<commit>>", norm_expected)
    canon_actual = COMMIT_PLACEHOLDER_PATTERN.sub("<<commit>>", norm_actual)

    return canon_expected, canon_actual
