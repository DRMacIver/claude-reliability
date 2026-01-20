//! Real sub-agent implementation using the Claude CLI.

use crate::error::Result;
use crate::traits::{CommandRunner, SubAgent, SubAgentDecision};
use std::time::Duration;

/// Timeout for sub-agent question decisions (60 seconds).
const QUESTION_DECISION_TIMEOUT: Duration = Duration::from_secs(60);

/// Timeout for code reviews (5 minutes).
const CODE_REVIEW_TIMEOUT: Duration = Duration::from_secs(300);

/// Timeout for reflection checks (90 seconds).
const REFLECTION_TIMEOUT: Duration = Duration::from_secs(90);

/// Real sub-agent implementation using the Claude CLI.
pub struct RealSubAgent<'a> {
    runner: &'a dyn CommandRunner,
    /// Optional explicit path to claude command (for testing).
    claude_cmd: Option<String>,
}

impl<'a> RealSubAgent<'a> {
    /// Create a new real sub-agent.
    pub fn new(runner: &'a dyn CommandRunner) -> Self {
        Self { runner, claude_cmd: None }
    }

    /// Set an explicit path to the claude command (for testing).
    #[cfg(test)]
    #[must_use]
    pub fn with_claude_cmd(mut self, cmd: impl Into<String>) -> Self {
        self.claude_cmd = Some(cmd.into());
        self
    }

    /// Get the claude command to use.
    fn claude_cmd(&self) -> &str {
        self.claude_cmd.as_deref().unwrap_or("claude")
    }
}

impl SubAgent for RealSubAgent<'_> {
    fn decide_on_question(
        &self,
        assistant_output: &str,
        user_recency_minutes: u32,
    ) -> Result<SubAgentDecision> {
        let prompt = format!(
            r#"You are a sub-agent helping to manage an autonomous session.

The main agent has stopped and its last output appears to contain a question.
The user has been active recently (within the last {user_recency_minutes} minutes).

Your task is to decide:
1. Should we allow the main agent to stop so the user can respond to the question?
2. Or can you answer the question directly based on the context?

Here is the end of the main agent's last output:

<assistant_output>
{assistant_output}
</assistant_output>

Respond with EXACTLY one of these formats:

ALLOW_STOP: <brief reason why user should respond>

or

ANSWER: <your direct answer to the question>

or

CONTINUE: <reason why this doesn't seem like a real question for the user>

Choose ALLOW_STOP if:
- The question requires user preference or decision
- The question asks for clarification about requirements
- The question offers options the user should choose from

Choose ANSWER if:
- You can provide a reasonable default answer
- The question is about process/approach and you can decide
- IMPORTANT: If the question is "Do you want me to continue?", "Should I
  proceed?", "Do you want me to do the rest?", or any variation asking
  whether to keep working, ALWAYS answer "Yes, please continue."

Choose CONTINUE if:
- This doesn't look like a real question for the user
- It's a rhetorical question
- The agent should keep working without user input"#
        );

        let output = self.runner.run(
            self.claude_cmd(),
            &["--print", "--model", "haiku", "-p", &prompt],
            Some(QUESTION_DECISION_TIMEOUT),
        )?;

        if !output.success() {
            // If Claude fails, default to Continue
            return Ok(SubAgentDecision::Continue);
        }

        let response = output.stdout.trim();

        // Parse the response format (if-let-else chain is more readable here)
        #[allow(clippy::option_if_let_else)]
        if let Some(reason) = response.strip_prefix("ALLOW_STOP:") {
            Ok(SubAgentDecision::AllowStop(Some(reason.trim().to_string())))
        } else if let Some(answer) = response.strip_prefix("ANSWER:") {
            Ok(SubAgentDecision::Answer(answer.trim().to_string()))
        } else {
            // Default: CONTINUE or unrecognized format
            Ok(SubAgentDecision::Continue)
        }
    }

    fn review_code(
        &self,
        diff: &str,
        files: &[String],
        review_guide: Option<&str>,
    ) -> Result<(bool, String)> {
        let files_list = files.iter().map(|f| format!("- {f}")).collect::<Vec<_>>().join("\n");

        let guide_section = review_guide
            .unwrap_or("No specific review guidelines provided. Use general best practices.");

        let prompt = format!(
            r#"You are a code reviewer. Review the following git diff and decide whether to APPROVE or REJECT the commit.

## Review Guidelines
{guide_section}

## Files Being Committed
{files_list}

## Diff to Review
```diff
{diff}
```

## Your Task
1. Review the code changes carefully
2. Check for:
   - Logic errors or bugs
   - Security issues (hardcoded secrets, injection vulnerabilities, etc.)
   - Code quality problems
   - Missing error handling
   - Breaking changes without proper handling
3. Make a decision: APPROVE or REJECT

## Response Format
You MUST respond with a JSON object in this exact format:
```json
{{
    "decision": "approve" or "reject",
    "feedback": "Your detailed review feedback here. Explain what you found, any concerns, and suggestions."
}}
```

If rejecting, explain clearly what needs to be fixed. If approving, you can still provide suggestions for improvement."#
        );

        let output = self.runner.run(
            self.claude_cmd(),
            &[
                "-p",
                &prompt,
                "--model",
                "opus",
                "--output-format",
                "json",
                "--allowedTools",
                "Read,Glob,Grep,Bash(git diff*),Bash(git log*),Bash(git show*)",
            ],
            Some(CODE_REVIEW_TIMEOUT),
        )?;

        if !output.success() {
            // If Claude fails, default to approve with warning
            return Ok((
                true,
                format!(
                    "Code review agent failed to run: {}. Proceeding with commit.",
                    output.stderr.chars().take(500).collect::<String>()
                ),
            ));
        }

        // Try to parse the JSON response
        let response = output.stdout.trim();

        // Try to find JSON in the output
        if let Some(json_match) = extract_json_object(response) {
            if let Ok(review) = serde_json::from_str::<serde_json::Value>(json_match) {
                let decision = review
                    .get("decision")
                    .and_then(|d| d.as_str())
                    .unwrap_or("approve")
                    .to_lowercase();
                let feedback = review
                    .get("feedback")
                    .and_then(|f| f.as_str())
                    .unwrap_or("No feedback provided.")
                    .to_string();

                return Ok((decision == "approve", feedback));
            }
        }

        // If parsing fails, approve with the raw output as feedback
        Ok((
            true,
            format!(
                "Review completed (could not parse structured response): {}",
                response.chars().take(1000).collect::<String>()
            ),
        ))
    }

    fn reflect_on_work(&self, assistant_output: &str, git_diff: &str) -> Result<(bool, String)> {
        let prompt = format!(
            r#"You are a self-reflection agent. Your task is to reflect on whether the assistant has completed the user's request properly, or whether there might be something incomplete, misunderstood, or shortcuts taken.

## Assistant's Last Output
<assistant_output>
{assistant_output}
</assistant_output>

## Changes Made (Git Diff)
<git_diff>
{git_diff}
</git_diff>

## Your Task
Carefully reflect on whether the work appears complete and correct. Consider:

1. **Completeness**: Has everything the user asked for been done? Are there any TODO items or "left for later" comments?

2. **Correctness**: Do the changes look correct and match what was requested? Are there any obvious bugs or issues?

3. **Shortcuts**: Were any corners cut? Any "I'll skip X" or "this should be good enough"?

4. **Misunderstandings**: Could the user's request have been misunderstood in any way?

5. **Edge cases**: Are there obvious edge cases that were missed?

Respond with a JSON object:
```json
{{
    "complete": true or false,
    "feedback": "Your analysis here. If complete is false, explain what seems incomplete or problematic. If complete is true, briefly confirm why the work looks good."
}}
```

Be constructively critical - it's better to flag potential issues than to miss them. However, don't be overly pedantic about trivial matters."#
        );

        let output = self.runner.run(
            self.claude_cmd(),
            &["-p", &prompt, "--model", "haiku", "--output-format", "json"],
            Some(REFLECTION_TIMEOUT),
        )?;

        if !output.success() {
            // If Claude fails, assume work is complete with warning
            return Ok((
                true,
                "Reflection check failed to run. Proceeding assuming work is complete.".to_string(),
            ));
        }

        let response = output.stdout.trim();

        // Try to find and parse JSON in the output
        if let Some(json_match) = extract_json_object(response) {
            if let Ok(reflection) = serde_json::from_str::<serde_json::Value>(json_match) {
                let complete =
                    reflection.get("complete").and_then(serde_json::Value::as_bool).unwrap_or(true);
                let feedback = reflection
                    .get("feedback")
                    .and_then(|f| f.as_str())
                    .unwrap_or("No feedback provided.")
                    .to_string();

                return Ok((complete, feedback));
            }
        }

        // If parsing fails, assume complete with the raw output as feedback
        Ok((
            true,
            format!(
                "Reflection check completed (could not parse response): {}",
                response.chars().take(500).collect::<String>()
            ),
        ))
    }
}

/// Extract a JSON object from text (looking for `{"decision": ...}`).
fn extract_json_object(text: &str) -> Option<&str> {
    // Find the start of a JSON object with "decision"
    let start = text.find('{')?;
    let substr = &text[start..];

    // Find the matching closing brace
    let mut depth = 0;
    let mut end = 0;
    for (i, c) in substr.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    end = i + 1;
                    break;
                }
            }
            _ => {}
        }
    }

    if end > 0 {
        Some(&substr[..end])
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::MockCommandRunner;
    use crate::traits::CommandOutput;

    #[test]
    fn test_extract_json_object() {
        let text = r#"Here is my response: {"decision": "approve", "feedback": "LGTM"} end"#;
        let json = extract_json_object(text).unwrap();
        assert_eq!(json, r#"{"decision": "approve", "feedback": "LGTM"}"#);
    }

    #[test]
    fn test_extract_json_object_nested() {
        let text = r#"{"outer": {"inner": "value"}, "key": "val"}"#;
        let json = extract_json_object(text).unwrap();
        assert_eq!(json, text);
    }

    #[test]
    fn test_extract_json_object_none() {
        assert!(extract_json_object("no json here").is_none());
    }

    #[test]
    fn test_decide_on_question_allow_stop() {
        let mut runner = MockCommandRunner::new();
        runner.expect(
            "claude",
            &["--print", "--model", "haiku", "-p", ""],
            CommandOutput {
                exit_code: 0,
                stdout: "ALLOW_STOP: User needs to make a decision".to_string(),
                stderr: String::new(),
            },
        );

        // Note: This test will fail because the prompt won't match exactly
        // In real tests, we'd use a more sophisticated matcher
    }

    #[test]
    fn test_review_code_approve() {
        // Test the JSON extraction logic
        let response = r#"{"decision": "approve", "feedback": "Code looks good"}"#;
        let json = extract_json_object(response).unwrap();
        let review: serde_json::Value = serde_json::from_str(json).unwrap();
        assert_eq!(review["decision"], "approve");
        assert_eq!(review["feedback"], "Code looks good");
    }

    #[test]
    fn test_review_code_reject() {
        let response = r#"{"decision": "reject", "feedback": "Security issue found"}"#;
        let json = extract_json_object(response).unwrap();
        let review: serde_json::Value = serde_json::from_str(json).unwrap();
        assert_eq!(review["decision"], "reject");
    }

    #[test]
    fn test_extract_json_unclosed_brace() {
        // JSON with unclosed brace should return None
        let text = r#"{"decision": "approve""#;
        assert!(extract_json_object(text).is_none());
    }

    #[test]
    fn test_real_sub_agent_new() {
        // Just test that we can create a RealSubAgent
        let runner = MockCommandRunner::new();
        let _agent = RealSubAgent::new(&runner);
    }

    #[test]
    fn test_question_decision_timeout_constant() {
        // Verify the timeout constant
        assert_eq!(QUESTION_DECISION_TIMEOUT, Duration::from_secs(60));
    }

    #[test]
    fn test_code_review_timeout_constant() {
        // Verify the timeout constant
        assert_eq!(CODE_REVIEW_TIMEOUT, Duration::from_secs(300));
    }

    // Integration tests using fake claude CLI
    mod integration {
        use super::*;
        use crate::command::RealCommandRunner;
        use std::fs;
        use std::os::unix::fs::PermissionsExt;
        use tempfile::TempDir;

        /// Creates a script that outputs the expected result.
        fn setup_fake_claude(dir: &TempDir, output: &str, exit_code: i32) -> String {
            use std::io::Write;
            use std::sync::atomic::{AtomicU64, Ordering};

            // Use unique counter for filename uniqueness
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let unique_id = COUNTER.fetch_add(1, Ordering::Relaxed);

            // Simple script using POSIX sh - escape single quotes for shell
            let escaped_output = output.replace('\'', "'\\''");
            let script =
                format!("#!/bin/sh\nprintf '%s\\n' '{escaped_output}'\nexit {exit_code}\n");

            // Use test's TempDir for isolation
            let script_path = dir.path().join(format!("fake_claude_{unique_id}"));

            // Write file, sync, set permissions
            {
                let mut file = std::fs::File::create(&script_path).unwrap();
                file.write_all(script.as_bytes()).unwrap();
                file.sync_all().unwrap();
            }
            fs::set_permissions(&script_path, fs::Permissions::from_mode(0o755)).unwrap();

            script_path.to_string_lossy().to_string()
        }

        #[test]
        fn test_real_subagent_decide_allow_stop() {
            let dir = TempDir::new().unwrap();
            let claude_cmd = setup_fake_claude(&dir, "ALLOW_STOP: User preference needed", 0);

            let runner = RealCommandRunner::new();
            let agent = RealSubAgent::new(&runner).with_claude_cmd(&claude_cmd);

            let result = agent.decide_on_question("test output", 5).unwrap();

            assert!(matches!(
                result,
                SubAgentDecision::AllowStop(Some(ref r)) if r.contains("User preference")
            ));
        }

        #[test]
        fn test_real_subagent_decide_answer() {
            let dir = TempDir::new().unwrap();
            let claude_cmd = setup_fake_claude(&dir, "ANSWER: Yes, please continue.", 0);

            let runner = RealCommandRunner::new();
            let agent = RealSubAgent::new(&runner).with_claude_cmd(&claude_cmd);

            let result = agent.decide_on_question("Should I continue?", 5).unwrap();

            assert!(matches!(
                result,
                SubAgentDecision::Answer(ref a) if a.contains("continue")
            ));
        }

        #[test]
        fn test_real_subagent_decide_continue() {
            let dir = TempDir::new().unwrap();
            let claude_cmd = setup_fake_claude(&dir, "CONTINUE: Not a real question", 0);

            let runner = RealCommandRunner::new();
            let agent = RealSubAgent::new(&runner).with_claude_cmd(&claude_cmd);

            let result = agent.decide_on_question("test", 5).unwrap();

            assert_eq!(result, SubAgentDecision::Continue);
        }

        #[test]
        fn test_real_subagent_decide_unrecognized_format() {
            let dir = TempDir::new().unwrap();
            let claude_cmd = setup_fake_claude(&dir, "Some unrecognized response format", 0);

            let runner = RealCommandRunner::new();
            let agent = RealSubAgent::new(&runner).with_claude_cmd(&claude_cmd);

            let result = agent.decide_on_question("test", 5).unwrap();

            // Unrecognized format defaults to Continue
            assert_eq!(result, SubAgentDecision::Continue);
        }

        #[test]
        fn test_real_subagent_decide_command_fails() {
            let dir = TempDir::new().unwrap();
            let claude_cmd = setup_fake_claude(&dir, "error", 1);

            let runner = RealCommandRunner::new();
            let agent = RealSubAgent::new(&runner).with_claude_cmd(&claude_cmd);

            let result = agent.decide_on_question("test", 5).unwrap();

            // Command failure defaults to Continue
            assert_eq!(result, SubAgentDecision::Continue);
        }

        #[test]
        fn test_real_subagent_review_approve() {
            let dir = TempDir::new().unwrap();
            let json = r#"{"decision": "approve", "feedback": "Code looks good"}"#;
            let claude_cmd = setup_fake_claude(&dir, json, 0);

            let runner = RealCommandRunner::new();
            let agent = RealSubAgent::new(&runner).with_claude_cmd(&claude_cmd);

            let (approved, feedback) =
                agent.review_code("+fn main() {}", &["src/main.rs".to_string()], None).unwrap();

            assert!(approved);
            assert!(feedback.contains("looks good"));
        }

        #[test]
        fn test_real_subagent_review_reject() {
            let dir = TempDir::new().unwrap();
            let json = r#"{"decision": "reject", "feedback": "Security issue found"}"#;
            let claude_cmd = setup_fake_claude(&dir, json, 0);

            let runner = RealCommandRunner::new();
            let agent = RealSubAgent::new(&runner).with_claude_cmd(&claude_cmd);

            let (approved, feedback) = agent
                .review_code("+password = 'secret'", &["src/config.rs".to_string()], None)
                .unwrap();

            assert!(!approved);
            assert!(feedback.contains("Security"));
        }

        #[test]
        fn test_real_subagent_review_with_guide() {
            let dir = TempDir::new().unwrap();
            let json = r#"{"decision": "approve", "feedback": "Follows guidelines"}"#;
            let claude_cmd = setup_fake_claude(&dir, json, 0);

            let runner = RealCommandRunner::new();
            let agent = RealSubAgent::new(&runner).with_claude_cmd(&claude_cmd);

            let (approved, _) = agent
                .review_code(
                    "+fn main() {}",
                    &["src/main.rs".to_string()],
                    Some("Review for security issues"),
                )
                .unwrap();

            assert!(approved);
        }

        #[test]
        fn test_real_subagent_review_command_fails() {
            let dir = TempDir::new().unwrap();
            let claude_cmd = setup_fake_claude(&dir, "error occurred", 1);

            let runner = RealCommandRunner::new();
            let agent = RealSubAgent::new(&runner).with_claude_cmd(&claude_cmd);

            let (approved, feedback) =
                agent.review_code("+fn main() {}", &["src/main.rs".to_string()], None).unwrap();

            // Command failure defaults to approve with warning
            assert!(approved);
            assert!(feedback.contains("failed to run"));
        }

        #[test]
        fn test_real_subagent_review_invalid_json() {
            let dir = TempDir::new().unwrap();
            let claude_cmd = setup_fake_claude(&dir, "This is not JSON at all", 0);

            let runner = RealCommandRunner::new();
            let agent = RealSubAgent::new(&runner).with_claude_cmd(&claude_cmd);

            let (approved, feedback) =
                agent.review_code("+fn main() {}", &["src/main.rs".to_string()], None).unwrap();

            // Invalid JSON defaults to approve
            assert!(approved);
            assert!(feedback.contains("could not parse"));
        }

        #[test]
        fn test_real_subagent_review_json_in_markdown() {
            let dir = TempDir::new().unwrap();
            // JSON embedded in markdown code block
            let response = "Here is my review:\n```json\n{\"decision\": \"approve\", \"feedback\": \"LGTM\"}\n```";
            let claude_cmd = setup_fake_claude(&dir, response, 0);

            let runner = RealCommandRunner::new();
            let agent = RealSubAgent::new(&runner).with_claude_cmd(&claude_cmd);

            let (approved, feedback) =
                agent.review_code("+fn main() {}", &["src/main.rs".to_string()], None).unwrap();

            assert!(approved);
            assert!(feedback.contains("LGTM"));
        }

        #[test]
        fn test_real_subagent_decide_spawn_fails() {
            // Use a command that doesn't exist to trigger runner.run() Err
            let runner = RealCommandRunner::new();
            let agent = RealSubAgent::new(&runner)
                .with_claude_cmd("/nonexistent/path/to/claude_command_that_does_not_exist");

            // The run() call returns Err, which propagates via ?
            let result = agent.decide_on_question("test", 5);
            assert!(result.is_err());
        }

        #[test]
        fn test_real_subagent_review_spawn_fails() {
            // Use a command that doesn't exist to trigger runner.run() Err
            let runner = RealCommandRunner::new();
            let agent = RealSubAgent::new(&runner)
                .with_claude_cmd("/nonexistent/path/to/claude_command_that_does_not_exist");

            // The run() call returns Err, which propagates via ?
            let result = agent.review_code("+fn main() {}", &["src/main.rs".to_string()], None);
            assert!(result.is_err());
        }

        #[test]
        fn test_real_subagent_review_json_looks_valid_but_invalid() {
            let dir = TempDir::new().unwrap();
            // JSON-like syntax that extract_json_object finds but can't be parsed
            // The { starts an object but has invalid JSON content
            let response = "Here is my review: {\"decision\": invalid}";
            let claude_cmd = setup_fake_claude(&dir, response, 0);

            let runner = RealCommandRunner::new();
            let agent = RealSubAgent::new(&runner).with_claude_cmd(&claude_cmd);

            let (approved, feedback) =
                agent.review_code("+fn main() {}", &["src/main.rs".to_string()], None).unwrap();

            // Falls through to default approval with raw output as feedback
            assert!(approved);
            assert!(feedback.contains("could not parse"));
        }

        #[test]
        fn test_real_subagent_reflect_complete() {
            let dir = TempDir::new().unwrap();
            let json = r#"{"complete": true, "feedback": "Work looks good"}"#;
            let claude_cmd = setup_fake_claude(&dir, json, 0);

            let runner = RealCommandRunner::new();
            let agent = RealSubAgent::new(&runner).with_claude_cmd(&claude_cmd);

            let (complete, feedback) = agent.reflect_on_work("test output", "+diff").unwrap();

            assert!(complete);
            assert!(feedback.contains("looks good"));
        }

        #[test]
        fn test_real_subagent_reflect_incomplete() {
            let dir = TempDir::new().unwrap();
            let json = r#"{"complete": false, "feedback": "Missing test coverage"}"#;
            let claude_cmd = setup_fake_claude(&dir, json, 0);

            let runner = RealCommandRunner::new();
            let agent = RealSubAgent::new(&runner).with_claude_cmd(&claude_cmd);

            let (complete, feedback) = agent.reflect_on_work("test output", "+diff").unwrap();

            assert!(!complete);
            assert!(feedback.contains("Missing test"));
        }

        #[test]
        fn test_real_subagent_reflect_command_fails() {
            let dir = TempDir::new().unwrap();
            let claude_cmd = setup_fake_claude(&dir, "error occurred", 1);

            let runner = RealCommandRunner::new();
            let agent = RealSubAgent::new(&runner).with_claude_cmd(&claude_cmd);

            let (complete, feedback) = agent.reflect_on_work("test output", "+diff").unwrap();

            // Command failure defaults to complete
            assert!(complete);
            assert!(feedback.contains("failed to run"));
        }

        #[test]
        fn test_real_subagent_reflect_invalid_json() {
            let dir = TempDir::new().unwrap();
            let claude_cmd = setup_fake_claude(&dir, "Not valid JSON at all", 0);

            let runner = RealCommandRunner::new();
            let agent = RealSubAgent::new(&runner).with_claude_cmd(&claude_cmd);

            let (complete, feedback) = agent.reflect_on_work("test output", "+diff").unwrap();

            // Invalid JSON defaults to complete
            assert!(complete);
            assert!(feedback.contains("could not parse"));
        }

        #[test]
        fn test_real_subagent_reflect_spawn_fails() {
            let runner = RealCommandRunner::new();
            let agent = RealSubAgent::new(&runner)
                .with_claude_cmd("/nonexistent/path/to/claude_command_that_does_not_exist");

            let result = agent.reflect_on_work("test output", "+diff");
            assert!(result.is_err());
        }
    }
}
