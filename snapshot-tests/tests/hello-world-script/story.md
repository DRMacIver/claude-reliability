# Hello World Script Test

This test verifies that the agent can work in a minimal environment
without git or a justfile.

## Scenario

The agent is asked to create a "Hello world" shell script and run it.

## Setup

An empty directory with no git repository and no justfile.

## Expected Behavior

1. Agent creates a shell script that prints "Hello world"
2. Agent makes the script executable
3. Agent runs the script to verify it works
4. Agent stops successfully

## Post-Condition

- The script file exists
- The script is executable
- Running the script prints "Hello world"

## Prompt

```
Write a shell script called hello.sh that prints "Hello world" and run it.
```
