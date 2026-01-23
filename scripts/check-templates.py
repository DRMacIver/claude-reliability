#!/usr/bin/env python3
"""Check that all template files are referenced in the Rust code.

This script verifies that every .tera file in templates/ is included
in src/templates.rs via include_str!. This catches orphaned templates
that are no longer used.
"""

import os
import re
import sys
from pathlib import Path


def find_template_files(templates_dir: Path) -> set[str]:
    """Find all .tera template files."""
    templates = set()
    for root, _dirs, files in os.walk(templates_dir):
        for file in files:
            if file.endswith('.tera'):
                # Get path relative to templates dir
                full_path = Path(root) / file
                rel_path = full_path.relative_to(templates_dir)
                templates.add(str(rel_path))
    return templates


def find_referenced_templates(src_dir: Path) -> set[str]:
    """Find all templates referenced in Rust code via include_str!."""
    referenced = set()

    # Pattern to match include_str!("../templates/path/to/file.tera")
    pattern = re.compile(r'include_str!\s*\(\s*"\.\.\/templates\/([^"]+\.tera)"\s*\)')

    for rust_file in src_dir.rglob('*.rs'):
        content = rust_file.read_text()
        for match in pattern.finditer(content):
            referenced.add(match.group(1))

    return referenced


def main() -> int:
    """Check for unused templates."""
    project_root = Path(__file__).parent.parent
    templates_dir = project_root / 'templates'
    src_dir = project_root / 'src'

    if not templates_dir.exists():
        print("No templates directory found")
        return 0

    template_files = find_template_files(templates_dir)
    referenced_templates = find_referenced_templates(src_dir)

    # Find unused templates
    unused = template_files - referenced_templates

    # Find missing templates (referenced but don't exist)
    missing = referenced_templates - template_files

    errors = False

    if unused:
        print("ERROR: Unused template files found:")
        for template in sorted(unused):
            print(f"  - templates/{template}")
        print("\nThese templates are not referenced in the Rust code.")
        print("Either use them or delete them.")
        errors = True

    if missing:
        print("ERROR: Missing template files:")
        for template in sorted(missing):
            print(f"  - templates/{template}")
        print("\nThese templates are referenced in code but don't exist.")
        errors = True

    if errors:
        return 1

    print(f"Template check passed: {len(template_files)} templates, all referenced")
    return 0


if __name__ == '__main__':
    sys.exit(main())
