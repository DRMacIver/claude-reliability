---
name: Understanding User Intent
description: "This skill should be used before planning or implementing any non-trivial request. It provides a systematic approach to understanding what the user actually wants, not just what their words literally say. Prevents building technically-correct-but-useless implementations by identifying load-bearing features, surfacing obvious implications, and sanity-checking interpretations."
---

# Understanding User Intent

## The Problem This Solves

The most expensive failure mode in software work is building something that technically satisfies a request but is obviously useless to the person who asked for it. This happens when:

- You interpret a request literally and miss what any human would obviously infer
- You implement a feature but omit its central, load-bearing functionality
- You satisfy the letter of a request while violating its spirit
- You build something that "works" in a way no reasonable person would find acceptable

**Examples of this failure mode:**

- A "notification system" where notifications don't get delivered
- A "search feature" that exists as a UI element but returns nothing useful
- A "configuration file" that the application never reads
- A "test suite" where every test passes unconditionally
- Adding a skill to a project when the user clearly meant to add it to the plugin they're working on

These are not subtle bugs. They are failures to understand what was being asked for.

## Phase 1: Read the Request as a Human Would

Before doing anything else, read the request the way a colleague sitting next to you would understand it.

### Identify the Obvious Implications

Every request has explicit content and implicit expectations. A human hearing the request would immediately understand both. You must too.

**Ask yourself:** "If I described my plan to the person who made this request, would they say 'but obviously I also need X'?"

**Examples of obvious implications:**

- "Add a search feature" implies the search must return relevant results, not just exist as a UI element
- "Make this configurable" implies the configuration must actually affect behavior, and there must be a way to set it
- "Add logging" implies the logs must be readable and contain useful information, not just `log("here")`
- "Write a skill for this plugin" implies the skill goes in the plugin's skill directory, not somewhere else
- "Add error handling" implies errors must be communicated to users, not silently swallowed
- "Cache the results" implies the cache must be invalidated when data changes, or it's worse than no cache
- "Add a hook for X" implies the hook must actually be wired up and triggered, not just defined

**Technique: State the implicit requirements explicitly.** Before starting work, write down what you believe the user implicitly expects. This forces you to confront assumptions you might otherwise skip past.

### Identify the Load-Bearing Feature

Every request has one or two features that are the entire point. Everything else is scaffolding. If you get the scaffolding right but miss the load-bearing feature, you've built something useless.

**Ask yourself:** "If I removed this one aspect, would the whole thing become pointless?"

**Examples:**

- A "retry mechanism" that never actually retries — the retry IS the feature
- An "authentication system" that doesn't check credentials — the checking IS the feature
- A "pre-tool-use hook" that doesn't fire before the tool is used — the timing IS the feature
- A "validation step" that accepts everything — the rejection of bad input IS the feature
- A "migration script" that doesn't transform the data — the transformation IS the feature

**Technique: Name the load-bearing feature.** Before implementation, state in one sentence: "The central point of this request is ___." If your implementation doesn't include that, it's wrong regardless of what else it includes.

### Read the Context, Not Just the Words

Requests exist in a context. That context tells you things the words alone don't.

- **Where are you?** If you're in a plugin directory, work refers to the plugin. If you're in a web app, "add a page" means a web page.
- **What was the conversation about?** Prior discussion constrains interpretation. If you've been discussing testing, "add this" probably means add a test.
- **What just happened?** If the user just saw a failure, their request is about fixing that failure, even if they phrase it generally.
- **Who is the user?** A user who has been describing detailed architectural requirements expects detailed architectural work, not a minimal stub.

## Phase 2: Think Through the Details

Once you understand the intent, think through how to actually satisfy it — not just how to produce something that superficially resembles what was asked for.

### The "Would This Actually Work?" Test

For each major component of your planned implementation, ask: "If a user tried to use this right now, would it actually do what they need?"

This is different from "does the code compile" or "do the tests pass." It's asking whether the thing you're building would be recognized as useful by the person who asked for it.

**Common failure modes to watch for:**

- **Stub implementations**: Adding a function signature but not the body, or a body that returns a placeholder
- **Missing integration**: Building a component that works in isolation but isn't connected to anything
- **Wrong level of abstraction**: Building infrastructure for a problem when the user wanted a solution
- **Cosmetic compliance**: Making something that looks right (correct file structure, correct names) but doesn't actually function
- **Solving the easy version**: The user asked for something hard, and your plan is suspiciously simple because you've unconsciously substituted an easier problem

### Trace the Usage Path

Walk through exactly how the feature will be used, step by step:

1. What triggers this feature?
2. What input does it receive?
3. What does it do with that input?
4. What output does it produce?
5. Where does that output go?
6. What does the user see or experience?

If any step in this chain is broken or missing, the feature doesn't work. A feature that works through steps 1-4 but fails at step 5 is just as broken as one that fails at step 1.

### Consider the Context of Use

Features don't exist in isolation. They exist within a system and are used by people in specific contexts.

- **Where does this fit?** If adding to an existing system, your implementation must integrate with that system. A standalone solution that ignores the existing architecture is usually wrong.
- **What comes before and after?** Your feature is part of a workflow. What triggers it? What depends on its output?
- **What are the constraints?** Performance, compatibility, security, user expectations. Features that ignore constraints are features that don't work in practice.

## Phase 3: Sanity Check Your Understanding

Before implementing, verify that your understanding makes sense. This is not about whether the request itself is reasonable — it's about whether your interpretation of the request is what a reasonable person would understand by it.

### The Explanation Test

Explain your planned approach in plain language: "The user asked for X. I understand this to mean they want Y, which I'll implement by doing Z."

Now ask: **"Would the person who made this request agree that Y is what they meant by X?"**

If there's any doubt, you've found a gap. Either:

- Ask for clarification (preferred when the ambiguity is real)
- Choose the most obviously useful interpretation (when one interpretation is clearly more useful than others)

### The Outsider Test

Imagine showing your completed work to someone who knows what was requested but hasn't seen your implementation. Would they say:

- "Yes, that's what was asked for" — Good, proceed
- "That's technically what was asked for, but it's not useful" — You've missed the intent
- "That's not what was asked for at all" — You've misunderstood fundamentally
- "That's part of what was asked for" — You've missed scope

### Red Flags That You've Misunderstood

Watch for these signs that your interpretation has gone wrong:

- **Your plan ignores a key phrase in the request.** If the user said "add X to Y" and your plan doesn't touch Y, something is wrong.
- **You're solving a different (usually easier) problem.** If the user asked for something hard and your plan is surprisingly simple, you may be solving the wrong problem.
- **You're adding something but not connecting it.** New code that nothing calls, new config that nothing reads, new tests that nothing asserts.
- **The scope seems way too small or way too large.** If a reasonable request leads to a one-line change or a complete rewrite, reconsider your interpretation.
- **You're working in the wrong place.** If you're modifying files that have nothing to do with the feature's intended location, you may have misread the context.

## Phase 4: Verify After Implementation

After building the thing, verify it's what was actually wanted.

### Walk Through the Original Request

Re-read the original request word by word. For each element:

- Is this addressed in your implementation?
- Does your implementation match what was meant, not just what was literally said?
- Would the user recognize this as fulfilling their request?

### Test the Happy Path End-to-End

Don't just verify individual components. Verify the complete flow:

1. Start from the user's perspective
2. Trigger the feature the way the user would
3. Follow the entire path through to the result
4. Verify the result is what the user would expect

If you can't do this (because the feature isn't integrated, or the entry point doesn't exist, or the output goes nowhere), the feature isn't done.

### Check for Completeness

Common things that get forgotten:

- **Error cases**: What happens when things go wrong? Does the user see something useful?
- **Integration**: Does this connect to the rest of the system? Can other parts use it?
- **The hook is wired up**: If you added a hook, is it registered? Does it trigger? Does the output reach the model?
- **The skill is discoverable**: If you added a skill, can the system find it? Does it show up where expected?

### The "Would I Accept This?" Test

If you were the one who made the request, would you consider this response satisfactory? Not perfect — satisfactory. Would you feel your request was understood and addressed?

If the answer is "technically yes, but I'd be disappointed," the work isn't done.

## Quick Reference Checklist

Before starting implementation:

- [ ] I can state the central, load-bearing feature of this request in one sentence
- [ ] I've identified the obvious implications a human would infer
- [ ] I've traced the usage path from trigger to user-visible result
- [ ] My explanation of the plan would satisfy the person who made the request
- [ ] I'm working in the right place (right project, right module, right directory)

Before declaring completion:

- [ ] I've re-read the original request and checked each element
- [ ] The feature works end-to-end, not just in isolated components
- [ ] I haven't ignored any key phrase or requirement from the request
- [ ] Everything I added is connected and integrated (no dead code, no unregistered hooks, no orphaned files)
- [ ] A reasonable person would recognize this as fulfilling the request
