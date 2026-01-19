#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.10"
# ///
"""
Custom Code Coverage Check Script
=================================

PURPOSE:
This script provides a more nuanced coverage check than simple percentage thresholds.
LLVM's coverage instrumentation counts "regions" which sometimes includes closing
braces of control structures as separate coverage points. When using early returns
or match arms that don't fall through, these closing braces are marked as "uncovered"
even though they represent no actual logic.

WHY CLOSING BRACES ARE ALLOWED AS UNCOVERED:
---------------------------------------------

LLVM-cov uses "region-based" coverage tracking, not line-based. A "region" is a
contiguous block of code with the same execution count. The closing brace of a
control structure (if, match, loop, etc.) sometimes gets marked as a separate
region that is "unreachable" when:

1. All paths through the block end with early returns/breaks
2. Match arms return values without falling through
3. Loops that never complete normally (infinite loops with break)

Example of a false-positive uncovered closing brace:

    if some_condition {
        return Ok(result);  // <-- covered when condition is true
    }                       // <-- "uncovered" - LLVM marks this as unreachable
    // more code...         // <-- covered when condition is false

The closing brace at line 3 is structural syntax, not executable code. LLVM-cov
reports it as a separate region with 0 executions because control flow never
"falls through" the if block - it either returns inside or skips the block entirely.

This is a well-known limitation of LLVM's region-based coverage:
- https://github.com/rust-lang/rust/issues/84605
- https://github.com/taiki-e/cargo-llvm-cov/issues/370

SECURITY NOTE:
--------------
This script should only be modified to allow ADDITIONAL exclusions with EXTREME CAUTION.
Adding new patterns to the allowlist could mask actual coverage gaps.

APPROVED EXCLUSIONS (currently only structural syntax):
- Lines containing only: } ) ; , or combinations thereof (e.g., "}", "});", "}},")

DO NOT ADD without careful review:
- Error handling code (should be tested!)
- Panic paths (should be tested or documented!)
- Any actual logic
"""

from __future__ import annotations

import re
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path


@dataclass
class UncoveredLine:
    """Represents an uncovered line in the source code."""

    file: Path
    line_number: int
    content: str

    def is_structural_syntax_only(self) -> bool:
        """
        Check if this line contains only structural syntax (closing braces, etc.).

        Returns True for lines like: }  })  },  });  }};  etc.
        Returns False for: }else  } // comment  }foo  or any actual code
        """
        # Remove all structural characters and whitespace
        stripped = self.content.strip()
        cleaned = re.sub(r"[})\];,\s]", "", stripped)

        # If nothing remains after removing structural chars, it's just syntax
        # But the original must have had something (not be empty)
        return len(cleaned) == 0 and len(stripped) > 0


def run_coverage() -> Path:
    """Run cargo llvm-cov and generate lcov.info."""
    print("Running coverage analysis...")
    lcov_path = Path("lcov.info")

    # Use --format=terse for compact output (dots instead of test names)
    # Use -- to separate cargo-llvm-cov args from test runner args
    result = subprocess.run(
        [
            "cargo", "llvm-cov", "--lib", "--lcov", f"--output-path={lcov_path}",
            "--", "--format=terse"
        ],
        capture_output=True,
        text=True,
    )

    if result.returncode != 0:
        # Show stderr on failure
        print(f"ERROR: Coverage generation failed:\n{result.stderr}", file=sys.stderr)
        sys.exit(1)

    # Show a summary instead of all test output
    # Count tests from the terse output (dots)
    test_output = result.stderr  # cargo test outputs to stderr
    print("Tests completed successfully")

    if not lcov_path.exists():
        print("ERROR: lcov.info was not generated", file=sys.stderr)
        sys.exit(1)

    return lcov_path


def parse_lcov(lcov_path: Path) -> list[UncoveredLine]:
    """
    Parse lcov.info file to find uncovered lines.

    LCOV format:
        SF:<source file path>
        DA:<line number>,<execution count>
        ...
        end_of_record
    """
    uncovered: list[UncoveredLine] = []
    current_file: Path | None = None

    with lcov_path.open() as f:
        for line in f:
            line = line.strip()

            if line.startswith("SF:"):
                current_file = Path(line[3:])

            elif line.startswith("DA:") and current_file is not None:
                # DA:line_number,execution_count
                match = re.match(r"DA:(\d+),(\d+)", line)
                if match:
                    line_number = int(match.group(1))
                    exec_count = int(match.group(2))

                    if exec_count == 0:
                        # Get the actual content of the line
                        content = get_line_content(current_file, line_number)
                        uncovered.append(
                            UncoveredLine(
                                file=current_file,
                                line_number=line_number,
                                content=content,
                            )
                        )

            elif line == "end_of_record":
                current_file = None

    return uncovered


def get_line_content(file_path: Path, line_number: int) -> str:
    """Get the content of a specific line from a file."""
    try:
        with file_path.open() as f:
            for i, line in enumerate(f, 1):
                if i == line_number:
                    return line.rstrip("\n")
    except (OSError, IOError):
        pass
    return ""


def main() -> int:
    """Main entry point."""
    # Generate coverage
    lcov_path = run_coverage()

    # Parse uncovered lines
    uncovered = parse_lcov(lcov_path)

    if not uncovered:
        print("\n✓ 100% line coverage achieved!")
        return 0

    # Categorize uncovered lines
    structural_only: list[UncoveredLine] = []
    actual_code: list[UncoveredLine] = []

    for line in uncovered:
        if line.is_structural_syntax_only():
            structural_only.append(line)
        else:
            actual_code.append(line)

    # Report results
    print()
    print("Coverage Analysis Results")
    print("=========================")
    print()
    print(f"Uncovered closing braces (allowed): {len(structural_only)}")
    print(f"Uncovered code lines: {len(actual_code)}")
    print()

    if not actual_code:
        print("✓ All uncovered lines are structural closing braces.")
        print("  Coverage check PASSED!")
        print()
        print("Note: LLVM-cov reports these as uncovered due to how it tracks regions.")
        print("      These closing braces contain no logic and are safe to exclude.")
        return 0
    else:
        print("✗ Found uncovered CODE that is not just a closing brace:")
        print()
        for line in actual_code:
            # Show relative path for readability
            try:
                rel_path = line.file.relative_to(Path.cwd())
            except ValueError:
                rel_path = line.file
            print(f"  {rel_path}:{line.line_number}: {line.content.strip()}")
        print()
        print("ACTION REQUIRED:")
        print("  1. Add tests for the uncovered code, OR")
        print("  2. If truly untestable, document why in the code and update this script")
        print()
        print("DO NOT blindly add exceptions! Each exclusion must be justified.")
        return 1


if __name__ == "__main__":
    sys.exit(main())
