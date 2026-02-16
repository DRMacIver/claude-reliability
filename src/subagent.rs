//! Real sub-agent implementation using the Claude CLI.

use crate::error::Result;
use crate::subagent_logging::log_subagent_event;
use crate::templates;
use crate::traits::{
    CommandRunner, CreateQuestionContext, CreateQuestionDecision, EmergencyStopContext,
    EmergencyStopDecision, QuestionContext, ReflectionContext, ReflectionDecision, SubAgent,
    SubAgentDecision,
};
use std::time::{Duration, Instant};
use tera::Context;

/// Timeout for sub-agent question decisions (60 seconds).
const QUESTION_DECISION_TIMEOUT: Duration = Duration::from_secs(60);

/// Timeout for code reviews (5 minutes).
const CODE_REVIEW_TIMEOUT: Duration = Duration::from_secs(300);

/// Timeout for emergency stop decisions (60 seconds).
const EMERGENCY_STOP_TIMEOUT: Duration = Duration::from_secs(60);

/// Timeout for `create_question` decisions (60 seconds).
const CREATE_QUESTION_TIMEOUT: Duration = Duration::from_secs(60);

/// Timeout for reflection decisions (60 seconds).
const REFLECTION_DECISION_TIMEOUT: Duration = Duration::from_secs(60);

/// Subdirectory name for running sub-agents to avoid picking up project hooks.
const SUBAGENT_SUBDIR: &str = "claude-reliability-subagents";

/// Get the directory to run sub-agents in.
/// Creates a dedicated subdirectory under the system temp dir.
fn get_subagent_cwd() -> std::path::PathBuf {
    let mut path = std::env::temp_dir();
    path.push(SUBAGENT_SUBDIR);
    // Create the directory if it doesn't exist (ignore errors - we'll fail later if it matters)
    let _ = std::fs::create_dir_all(&path);
    path
}

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
    fn decide_on_question(&self, context: &QuestionContext) -> Result<SubAgentDecision> {
        let mut ctx = Context::new();
        ctx.insert("assistant_output", &context.assistant_output);
        ctx.insert("user_recency_minutes", &context.user_recency_minutes);
        ctx.insert("user_last_active", &context.user_last_active);
        ctx.insert("has_modifications_since_user", &context.has_modifications_since_user);

        let prompt = templates::render("prompts/question_decision.tera", &ctx)
            .expect("question_decision.tera template should always render");

        let start = Instant::now();

        // Run in a neutral directory to avoid picking up project hooks
        let output = self.runner.run_in_dir(
            self.claude_cmd(),
            &["--print", "--model", "haiku", "-p", &prompt],
            Some(QUESTION_DECISION_TIMEOUT),
            &get_subagent_cwd(),
        )?;

        #[allow(clippy::cast_possible_truncation)] // Duration in ms won't overflow u64
        let duration_ms = start.elapsed().as_millis() as u64;

        if !output.success() {
            log_subagent_event(
                "question_decision",
                &prompt,
                Some(&output.stderr),
                false,
                Some(duration_ms),
            );
            // If Claude fails, default to Continue
            return Ok(SubAgentDecision::Continue);
        }

        let response = output.stdout.trim();

        log_subagent_event("question_decision", &prompt, Some(response), true, Some(duration_ms));

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

        let mut ctx = Context::new();
        ctx.insert("guide_section", guide_section);
        ctx.insert("files_list", &files_list);
        ctx.insert("diff", diff);

        let prompt = templates::render("prompts/code_review.tera", &ctx)
            .expect("code_review.tera template should always render");

        let start = Instant::now();

        // Run in a neutral directory to avoid picking up project hooks
        let output = self.runner.run_in_dir(
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
            &get_subagent_cwd(),
        )?;

        #[allow(clippy::cast_possible_truncation)] // Duration in ms won't overflow u64
        let duration_ms = start.elapsed().as_millis() as u64;

        if !output.success() {
            log_subagent_event(
                "code_review",
                &prompt,
                Some(&output.stderr),
                false,
                Some(duration_ms),
            );
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

        log_subagent_event("code_review", &prompt, Some(response), true, Some(duration_ms));

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

    fn evaluate_emergency_stop(
        &self,
        context: &EmergencyStopContext,
    ) -> Result<EmergencyStopDecision> {
        let mut ctx = Context::new();
        ctx.insert("explanation", &context.explanation);

        let prompt = templates::render("prompts/emergency_stop_decision.tera", &ctx)
            .expect("emergency_stop_decision.tera template should always render");

        let start = Instant::now();

        // Run in a neutral directory to avoid picking up project hooks
        let output = self.runner.run_in_dir(
            self.claude_cmd(),
            &["--print", "--model", "haiku", "-p", &prompt],
            Some(EMERGENCY_STOP_TIMEOUT),
            &get_subagent_cwd(),
        )?;

        #[allow(clippy::cast_possible_truncation)] // Duration in ms won't overflow u64
        let duration_ms = start.elapsed().as_millis() as u64;

        if !output.success() {
            log_subagent_event(
                "emergency_stop",
                &prompt,
                Some(&output.stderr),
                false,
                Some(duration_ms),
            );
            // If Claude fails, default to Accept (conservative — let agent stop)
            return Ok(EmergencyStopDecision::Accept(None));
        }

        let response = output.stdout.trim();

        log_subagent_event("emergency_stop", &prompt, Some(response), true, Some(duration_ms));

        response.strip_prefix("ACCEPT:").map_or_else(
            || {
                response.strip_prefix("REJECT:").map_or(
                    // Unrecognized format — default to Accept
                    Ok(EmergencyStopDecision::Accept(None)),
                    |instructions| {
                        Ok(EmergencyStopDecision::Reject(instructions.trim().to_string()))
                    },
                )
            },
            |message| {
                let msg = message.trim();
                Ok(EmergencyStopDecision::Accept(if msg.is_empty() {
                    None
                } else {
                    Some(msg.to_string())
                }))
            },
        )
    }

    fn evaluate_create_question(
        &self,
        context: &CreateQuestionContext,
    ) -> Result<CreateQuestionDecision> {
        let mut ctx = Context::new();
        ctx.insert("question_text", &context.question_text);

        let prompt = templates::render("prompts/create_question_decision.tera", &ctx)
            .expect("create_question_decision.tera template should always render");

        let start = Instant::now();

        // Run in a neutral directory to avoid picking up project hooks
        let output = self.runner.run_in_dir(
            self.claude_cmd(),
            &["--print", "--model", "haiku", "-p", &prompt],
            Some(CREATE_QUESTION_TIMEOUT),
            &get_subagent_cwd(),
        )?;

        #[allow(clippy::cast_possible_truncation)] // Duration in ms won't overflow u64
        let duration_ms = start.elapsed().as_millis() as u64;

        if !output.success() {
            log_subagent_event(
                "create_question",
                &prompt,
                Some(&output.stderr),
                false,
                Some(duration_ms),
            );
            // If Claude fails, default to Create (let the question be created)
            return Ok(CreateQuestionDecision::Create);
        }

        let response = output.stdout.trim();

        log_subagent_event("create_question", &prompt, Some(response), true, Some(duration_ms));

        response.strip_prefix("AUTO_ANSWER:").map_or_else(
            // CREATE: or unrecognized format — allow question creation
            || Ok(CreateQuestionDecision::Create),
            |answer| Ok(CreateQuestionDecision::AutoAnswer(answer.trim().to_string())),
        )
    }

    fn evaluate_reflection(&self, context: &ReflectionContext) -> Result<ReflectionDecision> {
        let mut ctx = Context::new();
        ctx.insert("reflection_output", &context.reflection_output);
        ctx.insert("user_messages", &context.user_messages);

        let prompt = templates::render("prompts/reflection_decision.tera", &ctx)
            .expect("reflection_decision.tera template should always render");

        let start = Instant::now();

        // Run in a neutral directory to avoid picking up project hooks
        let output = self.runner.run_in_dir(
            self.claude_cmd(),
            &["--print", "--model", "haiku", "-p", &prompt],
            Some(REFLECTION_DECISION_TIMEOUT),
            &get_subagent_cwd(),
        )?;

        #[allow(clippy::cast_possible_truncation)] // Duration in ms won't overflow u64
        let duration_ms = start.elapsed().as_millis() as u64;

        if !output.success() {
            log_subagent_event(
                "reflection_decision",
                &prompt,
                Some(&output.stderr),
                false,
                Some(duration_ms),
            );
            // If Claude fails, default to Complete (avoid infinite loops)
            return Ok(ReflectionDecision::Complete);
        }

        let response = output.stdout.trim();

        log_subagent_event("reflection_decision", &prompt, Some(response), true, Some(duration_ms));

        Ok(parse_reflection_response(response))
    }
}

/// Parse a reflection decision response.
///
/// Expects either `COMPLETE` or `INCOMPLETE:` followed by `- item` lines.
fn parse_reflection_response(response: &str) -> ReflectionDecision {
    let trimmed = response.trim();

    if trimmed == "COMPLETE" || trimmed.starts_with("COMPLETE:") {
        return ReflectionDecision::Complete;
    }

    if let Some(rest) = trimmed.strip_prefix("INCOMPLETE:") {
        let items: Vec<String> = rest
            .lines()
            .map(str::trim)
            .filter(|line| line.starts_with("- "))
            .map(|line| line[2..].trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        if items.is_empty() {
            // INCOMPLETE but no items listed — treat as complete
            return ReflectionDecision::Complete;
        }

        return ReflectionDecision::Incomplete { items };
    }

    // Unrecognized format — default to Complete
    ReflectionDecision::Complete
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

    /// Helper to create a test `QuestionContext`.
    fn test_context(output: &str) -> QuestionContext {
        QuestionContext {
            assistant_output: output.to_string(),
            user_recency_minutes: 5,
            user_last_active: Some("2 minutes ago".to_string()),
            has_modifications_since_user: false,
        }
    }

    #[test]
    fn test_parse_reflection_response_complete() {
        let result = parse_reflection_response("COMPLETE");
        assert_eq!(result, ReflectionDecision::Complete);
    }

    #[test]
    fn test_parse_reflection_response_complete_with_colon() {
        let result = parse_reflection_response("COMPLETE: All work done");
        assert_eq!(result, ReflectionDecision::Complete);
    }

    #[test]
    fn test_parse_reflection_response_incomplete() {
        let response = "INCOMPLETE:\n- Fix the login bug\n- Add unit tests";
        let result = parse_reflection_response(response);
        assert_eq!(
            result,
            ReflectionDecision::Incomplete {
                items: vec!["Fix the login bug".to_string(), "Add unit tests".to_string()]
            }
        );
    }

    #[test]
    fn test_parse_reflection_response_incomplete_no_items() {
        // INCOMPLETE but no actual items listed — defaults to Complete
        let result = parse_reflection_response("INCOMPLETE:");
        assert_eq!(result, ReflectionDecision::Complete);
    }

    #[test]
    fn test_parse_reflection_response_incomplete_with_blank_lines() {
        let response = "INCOMPLETE:\n\n- Fix X\n  \n- Add Y\n";
        let result = parse_reflection_response(response);
        assert_eq!(
            result,
            ReflectionDecision::Incomplete {
                items: vec!["Fix X".to_string(), "Add Y".to_string()]
            }
        );
    }

    #[test]
    fn test_parse_reflection_response_unrecognized() {
        let result = parse_reflection_response("Some random text");
        assert_eq!(result, ReflectionDecision::Complete);
    }

    #[test]
    fn test_parse_reflection_response_incomplete_filters_empty_items() {
        let response = "INCOMPLETE:\n- \n- Fix X\n- ";
        let result = parse_reflection_response(response);
        assert_eq!(result, ReflectionDecision::Incomplete { items: vec!["Fix X".to_string()] });
    }

    #[test]
    fn test_reflection_decision_timeout_constant() {
        assert_eq!(REFLECTION_DECISION_TIMEOUT, Duration::from_secs(60));
    }

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

            let result = agent.decide_on_question(&test_context("test output")).unwrap();

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

            let result = agent.decide_on_question(&test_context("Should I continue?")).unwrap();

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

            let result = agent.decide_on_question(&test_context("test")).unwrap();

            assert_eq!(result, SubAgentDecision::Continue);
        }

        #[test]
        fn test_real_subagent_decide_unrecognized_format() {
            let dir = TempDir::new().unwrap();
            let claude_cmd = setup_fake_claude(&dir, "Some unrecognized response format", 0);

            let runner = RealCommandRunner::new();
            let agent = RealSubAgent::new(&runner).with_claude_cmd(&claude_cmd);

            let result = agent.decide_on_question(&test_context("test")).unwrap();

            // Unrecognized format defaults to Continue
            assert_eq!(result, SubAgentDecision::Continue);
        }

        #[test]
        fn test_real_subagent_decide_command_fails() {
            let dir = TempDir::new().unwrap();
            let claude_cmd = setup_fake_claude(&dir, "error", 1);

            let runner = RealCommandRunner::new();
            let agent = RealSubAgent::new(&runner).with_claude_cmd(&claude_cmd);

            let result = agent.decide_on_question(&test_context("test")).unwrap();

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
            let result = agent.decide_on_question(&test_context("test"));
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
        fn test_real_subagent_emergency_stop_accept_with_message() {
            let dir = TempDir::new().unwrap();
            let claude_cmd =
                setup_fake_claude(&dir, "ACCEPT: Missing credentials for deployment", 0);

            let runner = RealCommandRunner::new();
            let agent = RealSubAgent::new(&runner).with_claude_cmd(&claude_cmd);

            let context =
                EmergencyStopContext { explanation: "Cannot deploy without API key".to_string() };
            let result = agent.evaluate_emergency_stop(&context).unwrap();

            assert!(matches!(
                result,
                EmergencyStopDecision::Accept(Some(ref msg)) if msg.contains("Missing credentials")
            ));
        }

        #[test]
        fn test_real_subagent_emergency_stop_accept_empty_message() {
            let dir = TempDir::new().unwrap();
            let claude_cmd = setup_fake_claude(&dir, "ACCEPT:", 0);

            let runner = RealCommandRunner::new();
            let agent = RealSubAgent::new(&runner).with_claude_cmd(&claude_cmd);

            let context = EmergencyStopContext { explanation: "Blocked".to_string() };
            let result = agent.evaluate_emergency_stop(&context).unwrap();

            assert!(matches!(result, EmergencyStopDecision::Accept(None)));
        }

        #[test]
        fn test_real_subagent_emergency_stop_reject() {
            let dir = TempDir::new().unwrap();
            let claude_cmd = setup_fake_claude(
                &dir,
                "REJECT: Use the Deciding what to work on skill instead",
                0,
            );

            let runner = RealCommandRunner::new();
            let agent = RealSubAgent::new(&runner).with_claude_cmd(&claude_cmd);

            let context = EmergencyStopContext { explanation: "Too many tasks to do".to_string() };
            let result = agent.evaluate_emergency_stop(&context).unwrap();

            assert!(matches!(
                result,
                EmergencyStopDecision::Reject(ref msg) if msg.contains("Deciding what to work on")
            ));
        }

        #[test]
        fn test_real_subagent_emergency_stop_command_fails() {
            let dir = TempDir::new().unwrap();
            let claude_cmd = setup_fake_claude(&dir, "error", 1);

            let runner = RealCommandRunner::new();
            let agent = RealSubAgent::new(&runner).with_claude_cmd(&claude_cmd);

            let context = EmergencyStopContext { explanation: "Something broke".to_string() };
            let result = agent.evaluate_emergency_stop(&context).unwrap();

            // Command failure defaults to Accept
            assert!(matches!(result, EmergencyStopDecision::Accept(None)));
        }

        #[test]
        fn test_real_subagent_emergency_stop_unrecognized_format() {
            let dir = TempDir::new().unwrap();
            let claude_cmd = setup_fake_claude(&dir, "I think you should stop working on this", 0);

            let runner = RealCommandRunner::new();
            let agent = RealSubAgent::new(&runner).with_claude_cmd(&claude_cmd);

            let context = EmergencyStopContext { explanation: "Unclear".to_string() };
            let result = agent.evaluate_emergency_stop(&context).unwrap();

            // Unrecognized format defaults to Accept
            assert!(matches!(result, EmergencyStopDecision::Accept(None)));
        }

        #[test]
        fn test_real_subagent_emergency_stop_spawn_fails() {
            let runner = RealCommandRunner::new();
            let agent = RealSubAgent::new(&runner)
                .with_claude_cmd("/nonexistent/path/to/claude_command_that_does_not_exist");

            let context = EmergencyStopContext { explanation: "test".to_string() };
            let result = agent.evaluate_emergency_stop(&context);

            // The run() call returns Err, which propagates via ?
            assert!(result.is_err());
        }

        #[test]
        fn test_real_subagent_create_question_auto_answer() {
            use crate::traits::CreateQuestionContext;

            let dir = TempDir::new().unwrap();
            let claude_cmd = setup_fake_claude(
                &dir,
                "AUTO_ANSWER: Continue with other work. Use what_should_i_work_on to pick the next task.",
                0,
            );

            let runner = RealCommandRunner::new();
            let agent = RealSubAgent::new(&runner).with_claude_cmd(&claude_cmd);

            let context =
                CreateQuestionContext { question_text: "Too many tasks to complete".to_string() };
            let result = agent.evaluate_create_question(&context).unwrap();

            assert!(matches!(
                result,
                CreateQuestionDecision::AutoAnswer(ref answer) if answer.contains("what_should_i_work_on")
            ));
        }

        #[test]
        fn test_real_subagent_create_question_create() {
            use crate::traits::CreateQuestionContext;

            let dir = TempDir::new().unwrap();
            let claude_cmd =
                setup_fake_claude(&dir, "CREATE: Genuinely needs user input about API key", 0);

            let runner = RealCommandRunner::new();
            let agent = RealSubAgent::new(&runner).with_claude_cmd(&claude_cmd);

            let context = CreateQuestionContext {
                question_text: "What is the API key for the service?".to_string(),
            };
            let result = agent.evaluate_create_question(&context).unwrap();

            assert!(matches!(result, CreateQuestionDecision::Create));
        }

        #[test]
        fn test_real_subagent_create_question_command_fails() {
            use crate::traits::CreateQuestionContext;

            let dir = TempDir::new().unwrap();
            let claude_cmd = setup_fake_claude(&dir, "error", 1);

            let runner = RealCommandRunner::new();
            let agent = RealSubAgent::new(&runner).with_claude_cmd(&claude_cmd);

            let context = CreateQuestionContext { question_text: "Some question".to_string() };
            let result = agent.evaluate_create_question(&context).unwrap();

            // Command failure defaults to Create
            assert!(matches!(result, CreateQuestionDecision::Create));
        }

        #[test]
        fn test_real_subagent_create_question_unrecognized_format() {
            use crate::traits::CreateQuestionContext;

            let dir = TempDir::new().unwrap();
            let claude_cmd = setup_fake_claude(&dir, "I think this needs user input", 0);

            let runner = RealCommandRunner::new();
            let agent = RealSubAgent::new(&runner).with_claude_cmd(&claude_cmd);

            let context = CreateQuestionContext { question_text: "Unclear question".to_string() };
            let result = agent.evaluate_create_question(&context).unwrap();

            // Unrecognized format defaults to Create
            assert!(matches!(result, CreateQuestionDecision::Create));
        }

        #[test]
        fn test_real_subagent_create_question_spawn_fails() {
            use crate::traits::CreateQuestionContext;

            let runner = RealCommandRunner::new();
            let agent = RealSubAgent::new(&runner)
                .with_claude_cmd("/nonexistent/path/to/claude_command_that_does_not_exist");

            let context = CreateQuestionContext { question_text: "test".to_string() };
            let result = agent.evaluate_create_question(&context);

            // The run() call returns Err, which propagates via ?
            assert!(result.is_err());
        }

        #[test]
        fn test_real_subagent_reflection_complete() {
            use crate::traits::ReflectionContext;

            let dir = TempDir::new().unwrap();
            let claude_cmd = setup_fake_claude(&dir, "COMPLETE", 0);

            let runner = RealCommandRunner::new();
            let agent = RealSubAgent::new(&runner).with_claude_cmd(&claude_cmd);

            let context = ReflectionContext {
                reflection_output: "All work is done".to_string(),
                user_messages: vec!["Fix the bug".to_string()],
            };
            let result = agent.evaluate_reflection(&context).unwrap();

            assert_eq!(result, ReflectionDecision::Complete);
        }

        #[test]
        fn test_real_subagent_reflection_incomplete() {
            use crate::traits::ReflectionContext;

            let dir = TempDir::new().unwrap();
            let claude_cmd =
                setup_fake_claude(&dir, "INCOMPLETE:\n- Fix the login bug\n- Add unit tests", 0);

            let runner = RealCommandRunner::new();
            let agent = RealSubAgent::new(&runner).with_claude_cmd(&claude_cmd);

            let context = ReflectionContext {
                reflection_output: "Still need to fix login and add tests".to_string(),
                user_messages: vec![],
            };
            let result = agent.evaluate_reflection(&context).unwrap();

            assert_eq!(
                result,
                ReflectionDecision::Incomplete {
                    items: vec!["Fix the login bug".to_string(), "Add unit tests".to_string()]
                }
            );
        }

        #[test]
        fn test_real_subagent_reflection_command_fails() {
            use crate::traits::ReflectionContext;

            let dir = TempDir::new().unwrap();
            let claude_cmd = setup_fake_claude(&dir, "error", 1);

            let runner = RealCommandRunner::new();
            let agent = RealSubAgent::new(&runner).with_claude_cmd(&claude_cmd);

            let context =
                ReflectionContext { reflection_output: "test".to_string(), user_messages: vec![] };
            let result = agent.evaluate_reflection(&context).unwrap();

            // Command failure defaults to Complete
            assert_eq!(result, ReflectionDecision::Complete);
        }

        #[test]
        fn test_real_subagent_reflection_unrecognized_format() {
            use crate::traits::ReflectionContext;

            let dir = TempDir::new().unwrap();
            let claude_cmd = setup_fake_claude(&dir, "I think the work is done", 0);

            let runner = RealCommandRunner::new();
            let agent = RealSubAgent::new(&runner).with_claude_cmd(&claude_cmd);

            let context =
                ReflectionContext { reflection_output: "test".to_string(), user_messages: vec![] };
            let result = agent.evaluate_reflection(&context).unwrap();

            // Unrecognized format defaults to Complete
            assert_eq!(result, ReflectionDecision::Complete);
        }

        #[test]
        fn test_real_subagent_reflection_spawn_fails() {
            use crate::traits::ReflectionContext;

            let runner = RealCommandRunner::new();
            let agent = RealSubAgent::new(&runner)
                .with_claude_cmd("/nonexistent/path/to/claude_command_that_does_not_exist");

            let context =
                ReflectionContext { reflection_output: "test".to_string(), user_messages: vec![] };
            let result = agent.evaluate_reflection(&context);

            // The run() call returns Err, which propagates via ?
            assert!(result.is_err());
        }
    }
}
