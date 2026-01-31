# claude-reliability

This is a plugin that contains a grab bag of my personal infrastructure for trying to get Claude to be something I can actually delegate real software development tasks to. It's idiosyncratic, a work in progress, only sometimes effective, and every time I try to use Claude without it now I get sad.

In theory if you install it, Claude will become magically better at software development, at the cost of potentially taking quite a lot longer (mostly, but not always, because it actually does what you ask it to do rather than stopping 10% of the way through).

I do not necessarily expect it to be useful to other people in its current state, but if you want to give it a go, I'm interested in hearing other people's experiences with it.

## Problems solved

Broadly speaking the problems I am trying to solve are that Claude doesn't do what it's told and what it does doesn't work. This plugin tries to guide it away from common shortcuts it tries to take, and to enforce that it actually does the work that it is told to do. Claude really doesn't like doing that, so it's at best a partial success, but it's a lot better than not using it.

It is particularly optimised for a problem I keep running into where I ask Claude to do something that will take a couple of hours, go away, and come back later to find that 20 minutes in it decided it had done enough work and stopped, or that the task that I had asked it to do was too hard and so it should implement a simpler and obviously useless version of it.

Key features:

1. Enforce that all work must be tested before it is committed, and committed before it is marked complete.
2. Code is reviewed before committing.
3. When Claude asks questions like "Would you like me to actually do the work that you just asked me to do?" the plugin automatically answers "Yes".
4. When Claude asks questions in the middle of its work that have reasonable default answers, the plugin automatically provides those answers rather than letting Claude stop working.
5. Keep track of work that has been explicitly requested to be completed, and block exit until that work has actually been completed.

There are a number of escape hatches and special cases in here to try to keep Claude also useful for normal interaction while this is going on. They even mostly work.

The plugin also provides a number of skills around general software development and how to do it properly. I'm not actually sure how useful they are, they're a bit speculative.

## Installation

**Step 1: Add the marketplace**
```
/plugin marketplace add DRMacIver/claude-reliability
```

**Step 2: Install the plugin**
```
/plugin install claude-reliability@claude-reliability-marketplace
```

The plugin will automatically download the pre-built binary from the latest GitHub release, or build from source if no release is available for your platform. You will need a rust compiler if you're building it from source.

## Supported platforms

| Platform | Method |
|----------|--------|
| Linux x86_64 | Pre-built release |
| Linux ARM64 | Pre-built release |
| macOS ARM64 | Pre-built release |
| Other Unix Platforms| Builds from source (requires or will install Rust) |

It probably could be made to work on Windows if you wanted it to, but it currently relies on bash scripts that would need a Windows equivalent. I imagine it works fine on WSL but I haven't tested it. 

## Development

This project is largely vibe-coded, with Claude doing the majority of the development (with mostly-careful oversight from me). In theory the following command will drop you into a claude code shell in a docker container with all of the reliability features present:

```bash
just develop
```

In practice, this command has only ever been tested on my computer and CI, so you'll probably run into issues. Again, please let me know if you do.

## License

MIT
