#!/usr/bin/env python3
"""
Pre-tool code review hook for git commits.

This hook runs before Bash commands and checks if the command is a git commit.
If the commit includes source code files, it invokes a Claude Opus sub-agent
to review the diff and either approve or reject with feedback.

The sub-agent has:
- Read access to the entire repository
- Read-only git commands
- No write permissions
- No hooks (to prevent recursion)

Review guidance comes from REVIEWGUIDE.md in the project root.
"""

from __future__ import annotations

import json
import os
import re
import subprocess
import sys
from pathlib import Path


# Source code file extensions (language-agnostic but comprehensive)
SOURCE_EXTENSIONS = {
    # Python
    ".py", ".pyx", ".pyi",
    # JavaScript/TypeScript
    ".js", ".jsx", ".ts", ".tsx", ".mjs", ".cjs",
    # Rust
    ".rs",
    # Go
    ".go",
    # Java/Kotlin/Scala
    ".java", ".kt", ".kts", ".scala",
    # C/C++
    ".c", ".h", ".cpp", ".hpp", ".cc", ".hh", ".cxx", ".hxx",
    # C#
    ".cs",
    # Ruby
    ".rb",
    # PHP
    ".php",
    # Swift/Objective-C
    ".swift", ".m", ".mm",
    # Web frameworks
    ".vue", ".svelte",
    # Shell scripts
    ".sh", ".bash", ".zsh",
    # Other
    ".pl", ".pm",  # Perl
    ".lua",
    ".r", ".R",  # R
    ".jl",  # Julia
    ".ex", ".exs",  # Elixir
    ".erl", ".hrl",  # Erlang
    ".hs", ".lhs",  # Haskell
    ".ml", ".mli",  # OCaml
    ".clj", ".cljs", ".cljc",  # Clojure
    ".f90", ".f95", ".f03",  # Fortran
    ".sql",
    ".proto",  # Protocol buffers
    ".graphql", ".gql",
}

# Source code directories (files in these are likely source code)
SOURCE_DIRECTORIES = {
    "src", "lib", "app", "pkg", "cmd", "internal", "core",
    "test", "tests", "spec", "specs", "__tests__",
    "components", "pages", "routes", "handlers", "services",
    "models", "views", "controllers", "utils", "helpers",
}

# Directories to always exclude
EXCLUDED_DIRECTORIES = {
    ".beads", ".claude", ".git", ".github", ".vscode",
    "node_modules", "vendor", "__pycache__", ".mypy_cache",
    "dist", "build", "target", ".next", ".nuxt",
    "coverage", ".pytest_cache", ".tox", "venv", ".venv",
    "eggs", "*.egg-info",
}


def is_source_code_file(filepath: str) -> bool:
    """Determine if a file is source code based on heuristics."""
    path = Path(filepath)

    # Check if in excluded directory
    parts = path.parts
    for part in parts:
        if part in EXCLUDED_DIRECTORIES or part.endswith(".egg-info"):
            return False

    # Check extension
    if path.suffix.lower() in SOURCE_EXTENSIONS:
        return True

    # Check if in source directory
    for part in parts:
        if part.lower() in SOURCE_DIRECTORIES:
            return True

    return False


def get_staged_files() -> list[str]:
    """Get list of files staged for commit."""
    try:
        result = subprocess.run(
            ["git", "diff", "--cached", "--name-only"],
            capture_output=True,
            text=True,
            timeout=10,
        )
        if result.returncode != 0:
            return []
        return [f.strip() for f in result.stdout.strip().split("\n") if f.strip()]
    except Exception:
        return []


def get_staged_diff() -> str:
    """Get the staged diff for review."""
    try:
        result = subprocess.run(
            ["git", "diff", "--cached", "--unified=5"],
            capture_output=True,
            text=True,
            timeout=30,
        )
        if result.returncode != 0:
            return ""
        return result.stdout
    except Exception:
        return ""


def load_review_guide() -> str:
    """Load REVIEWGUIDE.md if it exists."""
    guide_path = Path("REVIEWGUIDE.md")
    if guide_path.exists():
        try:
            return guide_path.read_text()
        except Exception:
            pass
    return ""


def run_review_agent(diff: str, files: list[str], review_guide: str) -> dict:
    """Run Claude Opus sub-agent to review the diff.

    Returns dict with:
        - decision: "approve" or "reject"
        - feedback: review comments
    """
    files_list = "\n".join(f"- {f}" for f in files)

    prompt = f"""You are a code reviewer. Review the following git diff and decide whether to APPROVE or REJECT the commit.

## Review Guidelines
{review_guide if review_guide else "No specific review guidelines provided. Use general best practices."}

## Files Being Committed
{files_list}

## Diff to Review
```diff
{diff}
```

## Your Task
1. Review the code changes carefully
2. Check for:
   - Logic errors or bugs
   - Security issues (hardcoded secrets, injection vulnerabilities, etc.)
   - Code quality problems
   - Missing error handling
   - Breaking changes without proper handling
3. Make a decision: APPROVE or REJECT

## Response Format
You MUST respond with a JSON object in this exact format:
```json
{{
    "decision": "approve" or "reject",
    "feedback": "Your detailed review feedback here. Explain what you found, any concerns, and suggestions."
}}
```

If rejecting, explain clearly what needs to be fixed. If approving, you can still provide suggestions for improvement.
"""

    try:
        # Run Claude in print mode with restricted permissions
        result = subprocess.run(
            [
                "claude",
                "-p", prompt,
                "--model", "opus",
                "--output-format", "json",
                "--allowedTools", "Read,Glob,Grep,Bash(git diff*),Bash(git log*),Bash(git show*)",
            ],
            capture_output=True,
            text=True,
            timeout=300,  # 5 minute timeout
            env={**os.environ, "ANTHROPIC_API_KEY": os.environ.get("ANTHROPIC_API_KEY", "")},
        )

        if result.returncode != 0:
            # If Claude fails, default to approve with warning
            return {
                "decision": "approve",
                "feedback": f"Code review agent failed to run: {result.stderr[:500]}. Proceeding with commit."
            }

        # Parse the JSON response
        try:
            # The output might have extra text, try to extract JSON
            output = result.stdout.strip()
            # Try to find JSON in the output
            json_match = re.search(r'\{[^{}]*"decision"[^{}]*\}', output, re.DOTALL)
            if json_match:
                review_result = json.loads(json_match.group())
            else:
                review_result = json.loads(output)

            decision = review_result.get("decision", "approve").lower()
            feedback = review_result.get("feedback", "No feedback provided.")

            return {
                "decision": decision,
                "feedback": feedback,
            }
        except (json.JSONDecodeError, KeyError):
            # If parsing fails, approve with the raw output as feedback
            return {
                "decision": "approve",
                "feedback": f"Review completed (could not parse structured response): {output[:1000]}"
            }

    except subprocess.TimeoutExpired:
        return {
            "decision": "approve",
            "feedback": "Code review timed out after 5 minutes. Proceeding with commit."
        }
    except FileNotFoundError:
        return {
            "decision": "approve",
            "feedback": "Claude CLI not available for code review. Proceeding with commit."
        }
    except Exception as e:
        return {
            "decision": "approve",
            "feedback": f"Code review error: {str(e)[:200]}. Proceeding with commit."
        }


def main() -> int:
    # Only run in Claude Code sessions
    if not os.environ.get("CLAUDECODE"):
        return 0

    # Skip if code review is disabled
    if os.environ.get("SKIP_CODE_REVIEW"):
        return 0

    # Read hook input from stdin
    try:
        stdin_content = sys.stdin.read()
        if not stdin_content.strip():
            return 0
        hook_input = json.loads(stdin_content)
    except (json.JSONDecodeError, Exception):
        return 0

    # Check if this is a Bash tool call
    tool_name = hook_input.get("tool_name", "")
    if tool_name != "Bash":
        return 0

    # Get the command being executed
    tool_input = hook_input.get("tool_input", {})
    command = tool_input.get("command", "")

    # Check if this is a git commit command
    # Match: git commit, git commit -m, git commit -am, etc.
    # But not: git commit --amend (let that through without review)
    if not re.search(r'\bgit\s+commit\b', command):
        return 0

    # Skip review for --amend commits (usually small fixes)
    if "--amend" in command:
        return 0

    # Get staged files
    staged_files = get_staged_files()
    if not staged_files:
        return 0

    # Filter for source code files
    source_files = [f for f in staged_files if is_source_code_file(f)]
    if not source_files:
        # No source code files, allow the commit
        return 0

    # Get the diff for review
    diff = get_staged_diff()
    if not diff:
        return 0

    # Load review guide
    review_guide = load_review_guide()

    # Run the review
    print(f"Running code review for {len(source_files)} source file(s)...", file=sys.stderr)
    review = run_review_agent(diff, source_files, review_guide)

    decision = review.get("decision", "approve")
    feedback = review.get("feedback", "")

    if decision == "reject":
        # Block the commit
        print("", file=sys.stderr)
        print("=" * 60, file=sys.stderr)
        print("CODE REVIEW: REJECTED", file=sys.stderr)
        print("=" * 60, file=sys.stderr)
        print("", file=sys.stderr)
        print(feedback, file=sys.stderr)
        print("", file=sys.stderr)
        print("Please address the review feedback before committing.", file=sys.stderr)
        print("Set SKIP_CODE_REVIEW=1 to bypass (not recommended).", file=sys.stderr)
        print("", file=sys.stderr)
        return 2

    # Approved - provide feedback via additionalContext
    output = {
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "allow",
            "additionalContext": f"Code Review Feedback:\n{feedback}" if feedback else None,
        }
    }
    # Only include additionalContext if there's feedback
    if not feedback or feedback.startswith("Code review"):
        output["hookSpecificOutput"].pop("additionalContext", None)

    print(json.dumps(output))
    return 0


if __name__ == "__main__":
    sys.exit(main())
