"""Placeholder substitution system for fuzzy matching in snapshot tests.

Placeholders use the syntax <<name>> (e.g., <<commit 1>>, <<issue 1>>).
They allow tests to match variable content like git SHAs and issue IDs.
"""

from __future__ import annotations

import re
from dataclasses import dataclass, field

# Pattern to match placeholders: <<name>>
PLACEHOLDER_PATTERN = re.compile(r"<<([^>]+)>>")


@dataclass
class PlaceholderRegistry:
    """Tracks placeholder -> value mappings during test execution.

    Usage:
        registry = PlaceholderRegistry()

        # When processing expected text, store values for placeholders
        actual = "Commit abc123 created"
        expected = "Commit <<commit 1>> created"
        registry.match(expected, actual)  # Stores "commit 1" -> "abc123"

        # Later, same placeholder must match same value
        actual2 = "Pushed abc123 to remote"
        expected2 = "Pushed <<commit 1>> to remote"
        registry.match(expected2, actual2)  # Passes because value matches
    """

    values: dict[str, str] = field(default_factory=dict)

    def substitute(self, text: str, direction: str) -> str:
        """Substitute placeholders in text.

        Args:
            text: Text containing placeholders or actual values
            direction: Either 'expect' or 'actual'
                - 'expect': Replace <<name>> with stored value (for comparison)
                - 'actual': Replace actual values with <<name>> (for normalization)

        Returns:
            The substituted text
        """
        if direction == "expect":
            # Replace placeholders with their stored values
            def replacer(m: re.Match[str]) -> str:
                name = m.group(1)
                if name in self.values:
                    return self.values[name]
                # Keep the placeholder if no value stored yet
                return m.group(0)

            return PLACEHOLDER_PATTERN.sub(replacer, text)

        elif direction == "actual":
            # Replace actual values with placeholders
            result = text
            for name, value in self.values.items():
                result = result.replace(value, f"<<{name}>>")
            return result

        else:
            raise ValueError(f"Invalid direction: {direction}. Must be 'expect' or 'actual'")

    def match(self, expected: str, actual: str) -> bool:
        """Check if actual matches expected with placeholder substitution.

        On first occurrence of a placeholder, stores the corresponding actual value.
        On subsequent occurrences, verifies the actual value matches the stored one.

        Args:
            expected: Expected text, may contain <<name>> placeholders
            actual: Actual text from test execution

        Returns:
            True if actual matches expected (with placeholder substitution)
        """
        # Find all placeholders in expected
        placeholders = PLACEHOLDER_PATTERN.findall(expected)

        if not placeholders:
            # No placeholders, do exact match
            return expected == actual

        # Build a regex pattern from expected, capturing placeholder positions
        pattern = self._build_match_pattern(expected, placeholders)

        match = re.fullmatch(pattern, actual)
        if not match:
            return False

        # Extract captured values and verify/store
        for i, name in enumerate(placeholders):
            captured_value = match.group(i + 1)

            if name in self.values:
                # Verify it matches the stored value
                if self.values[name] != captured_value:
                    return False
            else:
                # Store the new value
                self.values[name] = captured_value

        return True

    def _build_match_pattern(self, expected: str, placeholders: list[str]) -> str:
        """Build a regex pattern from expected text with placeholders.

        Each placeholder becomes a capturing group that matches non-greedily.
        """
        # Escape regex special characters in the expected text
        parts = PLACEHOLDER_PATTERN.split(expected)

        pattern_parts = []
        placeholder_idx = 0

        for i, part in enumerate(parts):
            if i % 2 == 0:
                # Regular text - escape it
                pattern_parts.append(re.escape(part))
            else:
                # Placeholder name - create capturing group
                # Use non-greedy match for flexibility
                pattern_parts.append(r"(.+?)")
                placeholder_idx += 1

        return "".join(pattern_parts)

    def reset(self) -> None:
        """Clear all stored placeholder values."""
        self.values.clear()

    def get(self, name: str) -> str | None:
        """Get the stored value for a placeholder name."""
        return self.values.get(name)

    def set(self, name: str, value: str) -> None:
        """Manually set a placeholder value."""
        self.values[name] = value


def extract_placeholders(text: str) -> list[str]:
    """Extract all placeholder names from text.

    Args:
        text: Text that may contain <<name>> placeholders

    Returns:
        List of placeholder names (without the << >> delimiters)
    """
    return PLACEHOLDER_PATTERN.findall(text)


def normalize_with_placeholders(text: str, registry: PlaceholderRegistry) -> str:
    """Normalize text by replacing known values with their placeholder names.

    This is useful for creating expected text from actual output.

    Args:
        text: Actual text containing concrete values
        registry: Registry with value -> name mappings

    Returns:
        Text with values replaced by placeholders
    """
    return registry.substitute(text, "actual")
