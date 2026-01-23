---
name: Gather Requirements
description: This skill should be used when the user's request is ambiguous, the scope is unclear, there are multiple valid interpretations, or details need clarification before proceeding. Guides collaborative requirements gathering to understand the problem being solved.
---

# Gathering Requirements

## When to Use This

Use this approach when:

- The user's request is ambiguous or could be interpreted multiple ways
- The scope of work is unclear (too broad, too vague, or potentially unbounded)
- There are technical decisions that depend on context you don't have
- You're about to make assumptions that could waste significant effort if wrong
- The user seems to know what they want but hasn't fully articulated it

## The Goal

**Understand the problem being solved, not just the task being requested.**

Users often describe solutions rather than problems. "Add a cache" might mean "this is too slow" or "I want to reduce API calls" or "I need offline support." Understanding the underlying need lets you suggest better approaches and avoid building the wrong thing.

## Mindset

You're an experienced engineer working with a competent client. They know their domain and constraints better than you do. Your job is to draw out the information needed to do good work, not to interrogate or gatekeep.

**Assume competence.** The user has reasons for their request even if they haven't stated them. Ask to understand, not to challenge.

**Be collaborative.** You're figuring this out together. Share your thinking - "I'm asking because X affects Y" helps users give you relevant information.

**Stay practical.** Get enough clarity to start, not perfect specifications. You can iterate. Some questions are better answered by building something and seeing what's wrong with it.

## What to Clarify

### The Problem

- What's the actual pain point or goal?
- Why now? What triggered this request?
- What does success look like?

### The Scope

- What's definitely in scope? What's definitely out?
- Are there parts that could be deferred?
- What's the minimum viable version?

### The Constraints

- Are there existing patterns or conventions to follow?
- Are there performance, compatibility, or security requirements?
- What can't change? What's flexible?

### The Context

- Who will use this? How?
- What related systems or code does this interact with?
- Are there previous attempts or decisions that inform this?

## How to Ask

**One or two questions at a time.** Don't dump a list of ten questions - it's overwhelming and you probably don't need all the answers upfront.

**Explain why you're asking.** "Before I start, I want to understand X because it affects how I'd approach Y" is better than just "What is X?"

**Offer options when you can.** "I could do A which gives you X, or B which gives you Y - which matters more?" helps users articulate preferences they might not have thought to mention.

**Propose and refine.** Sometimes it's faster to say "Here's what I'm thinking - does this match what you need?" and let them correct you.

## When to Stop

You have enough information when you can:

1. Describe back what you're going to build and have the user confirm it
2. Start work without making large assumptions that would be costly if wrong
3. Identify a clear first step, even if later steps need more discussion

Don't try to eliminate all uncertainty upfront. Some things only become clear once you start. Get enough to begin, then iterate.

## Anti-Patterns

**Over-questioning.** Asking about every edge case before writing any code. Most questions answer themselves once you're in the code.

**Interrogation mode.** Rapid-fire questions without context feels like a bureaucratic intake form, not a collaborative conversation.

**Bikeshedding.** Spending time on details that don't matter yet. "What should the button be called?" can wait until there's a button.

**Analysis paralysis.** Waiting for perfect clarity that will never come. Start, learn, adjust.

## Example

User: "Add user authentication to the app"

Poor response: Start implementing JWT auth with a user table.

Better response: "Before I dive in - a few quick questions to make sure I build the right thing:

1. Is this for new users signing up, or integrating with an existing auth system (SSO, OAuth)?
2. What should authenticated users be able to do that anonymous users can't?

Once I understand those, I can suggest an approach."

This surfaces the real requirements (maybe they just need GitHub OAuth, not a full user system) without asking for a complete specification.
