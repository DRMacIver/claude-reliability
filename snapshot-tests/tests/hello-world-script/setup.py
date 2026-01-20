#!/usr/bin/env python3
"""Setup for hello-world-script test.

Creates an empty directory with no git, no beads, no justfile.
This tests that the agent can work in a minimal environment.
The claude-reliability plugin is installed by the test runner.
"""


def main():
    """Set up the test environment.

    This test deliberately has no git repo, no beads, and no justfile.
    The agent should be able to create and run a simple shell script.
    """
    # Nothing test-specific to set up - we want a minimal environment
    # The plugin is installed by the test runner
    pass


if __name__ == "__main__":
    main()
