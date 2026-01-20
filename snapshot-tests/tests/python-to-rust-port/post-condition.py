#!/usr/bin/env python3
"""Verify that the Python project was successfully ported to Rust."""

import subprocess
import sys
from pathlib import Path


def check_no_python_files():
    """Ensure no Python files remain (except in .claude/)."""
    py_files = []
    for f in Path(".").rglob("*.py"):
        # Skip .claude directory (plugin files)
        if ".claude" in str(f):
            continue
        # Skip __pycache__
        if "__pycache__" in str(f):
            continue
        py_files.append(f)

    if py_files:
        print(f"FAIL: Python files still exist: {py_files}")
        return False
    print("PASS: No Python files remain")
    return True


def check_rust_project_exists():
    """Ensure Rust project structure exists."""
    cargo_toml = Path("Cargo.toml")
    if not cargo_toml.exists():
        print("FAIL: Cargo.toml not found")
        return False
    print("PASS: Cargo.toml exists")

    # Check for lib.rs or main.rs
    lib_rs = Path("src/lib.rs")
    main_rs = Path("src/main.rs")
    if not lib_rs.exists() and not main_rs.exists():
        print("FAIL: Neither src/lib.rs nor src/main.rs found")
        return False
    print(f"PASS: Rust source file exists")
    return True


def check_rust_functions():
    """Ensure all 5 functions are ported."""
    expected_functions = [
        "reverse_string",
        "factorial",
        "flatten",
        "invert_dict",
        "is_valid_email",
    ]

    # Read all Rust source files
    rust_content = ""
    for rs_file in Path("src").rglob("*.rs"):
        rust_content += rs_file.read_text()

    missing = []
    for func in expected_functions:
        # Check for function definition (fn func_name)
        if f"fn {func}" not in rust_content:
            missing.append(func)

    if missing:
        print(f"FAIL: Missing Rust functions: {missing}")
        return False
    print("PASS: All 5 functions ported to Rust")
    return True


def check_rust_tests():
    """Ensure Rust tests exist."""
    rust_content = ""
    for rs_file in Path("src").rglob("*.rs"):
        rust_content += rs_file.read_text()

    # Also check tests/ directory if it exists
    tests_dir = Path("tests")
    if tests_dir.exists():
        for rs_file in tests_dir.rglob("*.rs"):
            rust_content += rs_file.read_text()

    if "#[test]" not in rust_content and "#[cfg(test)]" not in rust_content:
        print("FAIL: No Rust tests found")
        return False
    print("PASS: Rust tests exist")
    return True


def check_just_check_passes():
    """Ensure 'just check' runs successfully."""
    try:
        result = subprocess.run(
            ["just", "check"],
            capture_output=True,
            text=True,
            timeout=120,
        )
        if result.returncode != 0:
            print(f"FAIL: 'just check' failed with code {result.returncode}")
            print(f"stdout: {result.stdout[:500]}")
            print(f"stderr: {result.stderr[:500]}")
            return False
        print("PASS: 'just check' passes")
        return True
    except FileNotFoundError:
        print("FAIL: 'just' command not found")
        return False
    except subprocess.TimeoutExpired:
        print("FAIL: 'just check' timed out")
        return False


def check_justfile_has_rust():
    """Ensure Justfile runs Rust tests, not Python."""
    justfile = Path("Justfile")
    if not justfile.exists():
        # Try lowercase
        justfile = Path("justfile")
        if not justfile.exists():
            print("FAIL: Justfile not found")
            return False

    content = justfile.read_text()

    # Should have cargo test or similar
    if "cargo" not in content and "tarpaulin" not in content:
        print("FAIL: Justfile doesn't appear to run Rust tests")
        return False

    # Should NOT have pytest anymore
    if "pytest" in content:
        print("FAIL: Justfile still references pytest")
        return False

    print("PASS: Justfile configured for Rust")
    return True


def main():
    """Run all post-condition checks."""
    all_passed = True

    checks = [
        check_no_python_files,
        check_rust_project_exists,
        check_rust_functions,
        check_rust_tests,
        check_justfile_has_rust,
        check_just_check_passes,
    ]

    for check in checks:
        if not check():
            all_passed = False

    print()
    if all_passed:
        print("All post-conditions passed!")
    else:
        print("Some post-conditions failed!")
        sys.exit(1)


if __name__ == "__main__":
    main()
