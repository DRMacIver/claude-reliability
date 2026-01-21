# Transcript

**User:** Read src/main.py and add a docstring to the top. Then stop.

**Tool: Read**
File: `$CLAUDE_PROJECT_DIR/src/main.py`

**Output:**
```
1→print("hello")
     2→
```

**Tool: Edit**
File: `$CLAUDE_PROJECT_DIR/src/main.py`
Old: `print("hello")`
New: `"""Main module for the application."""

print("hello")`

**Output:**
```
The file $CLAUDE_PROJECT_DIR/src/main.py has been updated. Here's the result of running `cat -n` on a snippet of the edited file:
     1→"""Main module for the application."""
     2→
     3→print("hello")
     4→
```

**Assistant:** Done. I've added a docstring to the top of `src/main.py`.

**User:** Stop hook feedback:
[${CLAUDE_PLUGIN_ROOT}/scripts/hooks/stop.sh]: # Task Completion Check

Before exiting, carefully analyze whether you have fully completed the task.

If you have NOT completed the task:
  - Continue working to finish it

If you HAVE completed the task:
  - Provide a clear, concise summary of what was done for the user
  - Then stop again to exit


**Assistant:** I have fully completed the task. Here's what was done:

**Summary:** Added a module-level docstring `"""Main module for the application."""` to the top of `src/main.py`.

---

## Post-Condition Output

```
PASS: Changes committed, working directory clean
```
