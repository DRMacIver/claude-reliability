"""Plugin setup helpers for snapshot tests.

Provides utilities to install the claude-reliability plugin into test environments.
"""

from __future__ import annotations

import os
import shutil
from pathlib import Path


def get_project_root() -> Path:
    """Get the root directory of the claude-reliability project.

    Returns:
        Path to the project root (parent of snapshot-tests)
    """
    # This file is in snapshot-tests/snapshot_tests/plugin_setup.py
    # Project root is snapshot-tests/../
    return Path(__file__).parent.parent.parent


def install_plugin(target_dir: Path, home_dir: Path | None = None) -> None:
    """Install the claude-reliability plugin into a test directory.

    This sets up the plugin structure so that Claude Code will recognize
    and use the claude-reliability hooks.

    Args:
        target_dir: The test directory to install the plugin into
        home_dir: Optional HOME directory (for isolated tests). If provided,
                  the binary is also copied to $HOME/.claude-reliability/bin/
                  where ensure-binary.sh expects to find it.
    """
    project_root = get_project_root()

    # Create .claude directory structure
    claude_dir = target_dir / ".claude"
    claude_dir.mkdir(parents=True, exist_ok=True)

    # Create plugins directory
    plugins_dir = claude_dir / "plugins"
    plugins_dir.mkdir(exist_ok=True)

    # Create the plugin directory
    plugin_dir = plugins_dir / "claude-reliability"
    plugin_dir.mkdir(exist_ok=True)

    # Copy plugin.json
    plugin_json_src = project_root / ".claude-plugin" / "plugin.json"
    if plugin_json_src.exists():
        shutil.copy(plugin_json_src, plugin_dir / "plugin.json")

    # Copy scripts directory (preserving structure)
    scripts_src = project_root / "scripts"
    scripts_dst = plugin_dir / "scripts"
    if scripts_src.exists():
        if scripts_dst.exists():
            shutil.rmtree(scripts_dst)
        shutil.copytree(scripts_src, scripts_dst)

    # Copy commands directory if it exists
    commands_src = project_root / "commands"
    commands_dst = plugin_dir / "commands"
    if commands_src.exists():
        if commands_dst.exists():
            shutil.rmtree(commands_dst)
        shutil.copytree(commands_src, commands_dst)

    # Copy hooks directory (contains hooks.json)
    hooks_src = project_root / "hooks"
    hooks_dst = plugin_dir / "hooks"
    if hooks_src.exists():
        if hooks_dst.exists():
            shutil.rmtree(hooks_dst)
        shutil.copytree(hooks_src, hooks_dst)

    # Create bin directory and copy the binary
    bin_dir = claude_dir / "bin"
    bin_dir.mkdir(exist_ok=True)

    binary_src = project_root / "target" / "release" / "claude-reliability"
    if binary_src.exists():
        shutil.copy(binary_src, bin_dir / "claude-reliability")
        os.chmod(bin_dir / "claude-reliability", 0o755)

        # Also copy to the global cache location where ensure-binary.sh expects it
        # This is needed because the hook scripts call ensure-binary.sh which
        # looks in ~/.claude-reliability/bin/ for the binary
        effective_home = home_dir or Path.home()
        global_cache_dir = effective_home / ".claude-reliability" / "bin"
        global_cache_dir.mkdir(parents=True, exist_ok=True)
        global_binary = global_cache_dir / "claude-reliability"
        shutil.copy(binary_src, global_binary)
        os.chmod(global_binary, 0o755)

    # Update the plugin.json to point to the local scripts
    # The scripts use ${CLAUDE_PLUGIN_ROOT} which should resolve correctly
    # when the plugin is in .claude/plugins/claude-reliability/

    # Create a settings file to enable the plugin
    settings_dir = claude_dir / "settings.local.json"
    if not settings_dir.exists():
        settings_dir.write_text('{"enabledPlugins": ["claude-reliability"]}')

    # Run ensure-gitignore to create .gitignore entries
    # This is normally done by the session-start hook, but during replay no hooks run
    binary_path = bin_dir / "claude-reliability"
    if binary_path.exists():
        import subprocess
        subprocess.run(
            [str(binary_path), "ensure-gitignore"],
            cwd=target_dir,
            capture_output=True,
        )


def install_plugin_binary_only(target_dir: Path) -> None:
    """Install just the claude-reliability binary (no hooks).

    Use this for tests that don't need the full plugin setup.

    Args:
        target_dir: The test directory to install the binary into
    """
    project_root = get_project_root()

    # Create bin directory
    bin_dir = target_dir / ".claude" / "bin"
    bin_dir.mkdir(parents=True, exist_ok=True)

    # Copy the binary
    binary_src = project_root / "target" / "release" / "claude-reliability"
    if binary_src.exists():
        shutil.copy(binary_src, bin_dir / "claude-reliability")
        os.chmod(bin_dir / "claude-reliability", 0o755)
