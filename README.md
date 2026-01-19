# claude-reliability

Reliability hooks for Claude Code - prevents common mistakes and enforces quality standards.

## Installation

**Step 1: Add the marketplace**
```
/plugin marketplace add DRMacIver/claude-reliability
```

**Step 2: Install the plugin**
```
/plugin install claude-reliability@claude-reliability-marketplace
```

The plugin will automatically download the pre-built binary from the latest GitHub release, or build from source if no release is available for your platform.

## What the plugin provides

- **Stop hook**: Runs quality checks before Claude Code stops
- **PreToolUse hook (no-verify)**: Blocks `--no-verify` and similar flags in git commands
- **PreToolUse hook (code-review)**: Automated code review on commits

## Supported platforms

| Platform | Method |
|----------|--------|
| Linux x86_64 | Pre-built release |
| macOS ARM64 | Pre-built release |
| Linux ARM64 | Builds from source (requires Rust) |

## Development

```bash
just develop
```

## License

MIT
