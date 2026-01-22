# claude-reliability

Claude is great, except when it's not. I routinely got frustrated when it did amazing work and stopped at the 90% mark, or subverted or forgot my instructions, or otherwise just behaved extremely sloppily. As a result, I developed some elaborate personal infrastructure to get it to behave.

Then I got frustrated when I didn't have my elaborate personal infrastructure at work, so I decided to make it open source. This is a mostly-from-scratch rewrite of that, with a lot of the weirder bits fixed, some of the rest hardened up, and some attempt to make it useful to people who are not literally me.

**Warning**: One way in which this is highly specialised to my use cases is that it is likely to be quite token hungry. If you are on a cheap plan, please pay careful attention to usage as it may blow through your limits fast. I use 20xMax for most of my development and am happy to trade tokens for my time as a result if it gets me good software. If you have radically different preferences from me, you might still find this useful, but you might want to have a think about how to adapt it to you. I'm generally happy for it to be configurable, so if there are changes that would make it more useful to you please do feel free to request them or submit issues.

Main features:

1. A *lot* of guard rails on when Claude is allowed to stop. Tests and linting should pass (or whatever you want to configure here), code should be committed and pushed.
2. Self-review of code with another model before committing.
3. A just-keep-working mode which causes Claude to actually keep working until it's done.

This is currently alpha-grade software: I expect it to work well, except when it doesn't. Please let me know if you use it, and report any problems you encounter.

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

## Supported platforms

| Platform | Method |
|----------|--------|
| Linux x86_64 | Pre-built release |
| macOS ARM64 | Pre-built release |
| Other Unix Platforms| Builds from source (requires or will install Rust) |

## Development

This project is largely vibe-coded, with Claude doing the majority of the development (with careful oversight from me, and all of the tooling in place in this project to make that good and reliable). In theory the following command will drop you into a claude code shell in a docker container with all of the reliability features present:

```bash
just develop
```

In practice, this command has only ever been tested on my computer and CI, so you'll probably run into issues. Again, please let me know if you do.

## License

MIT
