//! Stop hook for autonomous mode and code quality checks.
//!
//! This hook runs when Claude attempts to stop/exit. It implements:
//! - Autonomous mode management with staleness detection
//! - Uncommitted changes detection and blocking
//! - Code quality checks on the diff
//! - Interactive question handling with sub-agent

use crate::analysis::{self, AnalysisResults};
use crate::beads;
use crate::error::Result;
use crate::git::{self, GitStatus};
use crate::hooks::HookInput;
use crate::question::{is_continue_question, looks_like_question, truncate_for_context};
use crate::session::{self, SessionConfig, STALENESS_THRESHOLD};
use crate::traits::{CommandRunner, SubAgent, SubAgentDecision};
use crate::transcript::{self, TranscriptInfo};
use std::collections::HashSet;
use std::path::Path;

/// Magic string that allows stopping when work is complete but human input is required.
pub const HUMAN_INPUT_REQUIRED: &str =
    "I have completed all work that I can and require human input to proceed.";

/// Magic string that allows stopping when encountering an unsolvable problem.
pub const PROBLEM_NEEDS_USER: &str = "I have run into a problem I can't solve without user input.";

/// Time window for considering user as "recently active" (minutes).
pub const USER_RECENCY_MINUTES: u32 = 5;

/// Configuration for the stop hook.
#[derive(Debug, Clone, Default)]
pub struct StopHookConfig {
    /// Skip quality checks (no-op by default until user configures).
    pub quality_check_enabled: bool,
    /// Command to run for quality checks.
    pub quality_check_command: Option<String>,
    /// Whether to require pushing before exit.
    pub require_push: bool,
    /// Whether we're in a repo critique mode (bypass hook).
    pub repo_critique_mode: bool,
}

/// Result of running the stop hook.
#[derive(Debug, Clone)]
pub struct StopHookResult {
    /// Whether to allow the stop (true = allow, false = block).
    pub allow_stop: bool,
    /// Exit code (0 = allow, 2 = block).
    pub exit_code: i32,
    /// Messages to display to stderr.
    pub messages: Vec<String>,
    /// Optional response to inject (from sub-agent).
    pub inject_response: Option<String>,
}

impl StopHookResult {
    /// Create an "allow" result.
    pub const fn allow() -> Self {
        Self { allow_stop: true, exit_code: 0, messages: Vec::new(), inject_response: None }
    }

    /// Create a "block" result.
    pub const fn block() -> Self {
        Self { allow_stop: false, exit_code: 2, messages: Vec::new(), inject_response: None }
    }

    /// Add a message to display.
    #[must_use]
    pub fn with_message(mut self, msg: impl Into<String>) -> Self {
        self.messages.push(msg.into());
        self
    }

    /// Add multiple messages.
    #[must_use]
    pub fn with_messages(mut self, msgs: impl IntoIterator<Item = String>) -> Self {
        self.messages.extend(msgs);
        self
    }

    /// Set an inject response.
    #[must_use]
    pub fn with_inject(mut self, response: impl Into<String>) -> Self {
        self.inject_response = Some(response.into());
        self
    }
}

/// Run the stop hook.
///
/// # Errors
///
/// Returns an error if git commands, sub-agent calls, or file operations fail.
#[allow(clippy::too_many_lines)] // Complex hook logic requires multiple checks
pub fn run_stop_hook(
    input: &HookInput,
    config: &StopHookConfig,
    runner: &dyn CommandRunner,
    sub_agent: &dyn SubAgent,
) -> Result<StopHookResult> {
    // Bypass the stop hook during repo critique
    if config.repo_critique_mode {
        return Ok(StopHookResult::allow());
    }

    // Parse transcript if available
    let transcript_info = input
        .transcript_path
        .as_ref()
        .and_then(|p| transcript::parse_transcript(Path::new(p)).ok())
        .unwrap_or_default();

    // Check if autonomous session is active
    let session_path = Path::new(session::SESSION_FILE_PATH);
    let session_config = session::parse_session_file(session_path)?;

    // Fast path: if no autonomous session and no git changes, allow immediate exit
    if session_config.is_none() {
        let git_status = git::check_uncommitted_changes(runner)?;
        if !git_status.uncommitted.has_changes() && !git_status.ahead_of_remote {
            return Ok(StopHookResult::allow());
        }
    }

    // Check for bypass strings in Claude's last output
    if let Some(ref output) = transcript_info.last_assistant_output {
        let has_complete_phrase = output.contains(HUMAN_INPUT_REQUIRED);
        let has_problem_phrase = output.contains(PROBLEM_NEEDS_USER);

        if has_complete_phrase || has_problem_phrase {
            // For the "work complete" phrase, check if there are remaining issues
            // The "problem" phrase allows exit even with open issues
            if has_complete_phrase && !has_problem_phrase && beads::is_beads_available(runner) {
                let open_count = beads::get_open_issues_count(runner)?;
                if open_count > 0 {
                    return Ok(StopHookResult::block()
                        .with_message("# Exit Phrase Rejected")
                        .with_message("")
                        .with_message(format!("There are {open_count} open issue(s) remaining."))
                        .with_message("")
                        .with_message("Please work on the remaining issues before exiting.")
                        .with_message("Run `bd ready` to see available work.")
                        .with_message("")
                        .with_message(
                            "If you've hit a blocker you can't resolve, use this phrase instead:",
                        )
                        .with_message("")
                        .with_message(format!("  \"{PROBLEM_NEEDS_USER}\"")));
                }
            }
            // Bypass allowed
            session::cleanup_session_file(session_path)?;
            let reason = if has_problem_phrase {
                "Problem requiring user input acknowledged. Allowing stop."
            } else {
                "Human input required acknowledged. Allowing stop."
            };
            return Ok(StopHookResult::allow().with_message(reason));
        }
    }

    // Check for uncommitted changes
    let git_status = git::check_uncommitted_changes(runner)?;

    if git_status.uncommitted.has_changes() {
        return handle_uncommitted_changes(
            &git_status,
            config,
            runner,
            &transcript_info,
            sub_agent,
        );
    }

    // Check if need to push
    if config.require_push && git_status.ahead_of_remote {
        return Ok(StopHookResult::block()
            .with_message("# Unpushed Commits")
            .with_message("")
            .with_message(format!(
                "You have {} commit(s) that haven't been pushed.",
                git_status.commits_ahead
            ))
            .with_message("")
            .with_message("Run `git push` to publish your changes."));
    }

    // Check if agent is asking a question and user is recently active
    if let Some(result) = check_interactive_question(&transcript_info, sub_agent)? {
        return Ok(result);
    }

    // Check autonomous mode
    if let Some(mut session) = session_config {
        return handle_autonomous_mode(&mut session, config, runner, sub_agent);
    }

    // Not in autonomous mode - run quality checks if enabled
    if config.quality_check_enabled {
        if let Some(ref cmd) = config.quality_check_command {
            let output = runner.run("sh", &["-c", cmd], None)?;
            if !output.success() {
                return Ok(StopHookResult::block()
                    .with_message("# Quality Gates Failed")
                    .with_message("")
                    .with_message("Quality checks must pass before exiting.")
                    .with_message("")
                    .with_message(truncate_output(&output.combined_output(), 50)));
            }
        }
    }

    Ok(StopHookResult::allow())
}

/// Handle uncommitted changes.
#[allow(clippy::too_many_lines)] // Complex logic with many message variants
fn handle_uncommitted_changes(
    git_status: &GitStatus,
    config: &StopHookConfig,
    runner: &dyn CommandRunner,
    _transcript_info: &TranscriptInfo,
    _sub_agent: &dyn SubAgent,
) -> Result<StopHookResult> {
    let mut result = StopHookResult::block();

    // Check beads interaction if beads is available
    if beads::is_beads_available(runner) {
        let beads_status = beads::check_beads_interaction(runner)?;
        if !beads_status.has_interaction && !beads_status.already_warned {
            beads::mark_beads_warning_given()?;
            return Ok(result
                .with_message("# Beads Interaction Required")
                .with_message("")
                .with_message("You have uncommitted changes but haven't interacted with beads.")
                .with_message("")
                .with_message("In Claude Code sessions, work should be tracked in beads:")
                .with_message("  - Create an issue: `bd create \"Issue title\"`")
                .with_message("  - Claim work: `bd update <id> --status in_progress`")
                .with_message("  - Complete work: `bd close <id>`")
                .with_message("")
                .with_message(
                    "If this work genuinely doesn't need tracking, try stopping again.",
                ));
        }
    }

    // Analyze the diff for code quality issues
    let combined_diff = git::combined_diff(runner)?;
    let added_lines = git::parse_diff(&combined_diff);
    let analysis = analysis::analyze_diff(&added_lines);

    // Run quality checks if enabled
    let mut quality_output = String::new();
    let mut quality_passed = true;
    if config.quality_check_enabled {
        if let Some(ref cmd) = config.quality_check_command {
            result.messages.push("# Running Quality Checks...".to_string());
            result.messages.push(String::new());
            let output = runner.run("sh", &["-c", cmd], None)?;
            quality_passed = output.success();
            quality_output = output.combined_output();
        }
    }

    result.messages.push("# Uncommitted Changes Detected".to_string());
    result.messages.push(String::new());
    result.messages.push(format!("Cannot exit with {}.", git_status.uncommitted.description()));
    result.messages.push(String::new());

    // Show quality check results
    if !quality_passed {
        result.messages.push("## Quality Checks Failed".to_string());
        result.messages.push(String::new());
        result
            .messages
            .push("Quality gates did not pass. Fix issues before committing.".to_string());
        result.messages.push(String::new());
        if !quality_output.is_empty() {
            result.messages.push("### Output:".to_string());
            result.messages.push(String::new());
            result.messages.push(truncate_output(&quality_output, 50));
        }
    }

    // Show analysis results
    add_analysis_messages(&mut result, &analysis);

    // Show untracked files
    if !git_status.untracked_files.is_empty() {
        result.messages.push("## Untracked Files".to_string());
        result.messages.push(String::new());
        result.messages.push("The following files are not tracked by git:".to_string());
        result.messages.push(String::new());
        for (i, f) in git_status.untracked_files.iter().enumerate() {
            if i >= 10 {
                result
                    .messages
                    .push(format!("  ... and {} more", git_status.untracked_files.len() - 10));
                break;
            }
            result.messages.push(format!("  {f}"));
        }
        result.messages.push(String::new());
        result.messages.push("Either `git add` them or add them to .gitignore".to_string());
        result.messages.push(String::new());
    }

    // Instructions
    result.messages.push("Before stopping, please:".to_string());
    result.messages.push(String::new());
    result
        .messages
        .push("1. Run `git status` to check for files that should be gitignored".to_string());
    if config.quality_check_enabled {
        result.messages.push("2. Run quality checks to verify they pass".to_string());
    }
    result.messages.push("3. Stage your changes: `git add <files>`".to_string());
    result.messages.push("4. Commit with a descriptive message: `git commit -m '...'`".to_string());
    if config.require_push {
        result.messages.push("5. Push to remote: `git push`".to_string());
        result.messages.push(String::new());
        result.messages.push("Work is incomplete until `git push` succeeds.".to_string());
    }
    result.messages.push(String::new());
    result.messages.push("---".to_string());
    result.messages.push(String::new());
    result
        .messages
        .push("If you've hit a problem you cannot solve without user input:".to_string());
    result.messages.push(String::new());
    result.messages.push(format!("  \"{PROBLEM_NEEDS_USER}\""));

    Ok(result)
}

/// Add analysis messages to the result.
fn add_analysis_messages(result: &mut StopHookResult, analysis: &AnalysisResults) {
    if !analysis.suppression_violations.is_empty() {
        result.messages.push("## Error Suppression Detected".to_string());
        result.messages.push(String::new());
        result.messages.push("The following error suppressions were added:".to_string());
        result.messages.push(String::new());
        for v in &analysis.suppression_violations {
            result.messages.push(v.format());
        }
        result.messages.push(String::new());
        result
            .messages
            .push("Fix the underlying issues instead of suppressing errors.".to_string());
        result.messages.push(String::new());
    }

    if !analysis.empty_except_violations.is_empty() {
        result.messages.push("## Empty Exception Handlers Detected".to_string());
        result.messages.push(String::new());
        result.messages.push("The following empty except blocks were added:".to_string());
        result.messages.push(String::new());
        for v in &analysis.empty_except_violations {
            result.messages.push(v.format());
        }
        result.messages.push(String::new());
        result.messages.push("Handle exceptions properly or re-raise them.".to_string());
        result.messages.push(String::new());
    }

    if !analysis.secret_violations.is_empty() {
        result.messages.push("## SECURITY: Hardcoded Secrets Detected".to_string());
        result.messages.push(String::new());
        result
            .messages
            .push("The following secrets/tokens were found in staged changes:".to_string());
        result.messages.push(String::new());
        for v in &analysis.secret_violations {
            result.messages.push(v.format());
        }
        result.messages.push(String::new());
        result
            .messages
            .push("NEVER commit secrets. Use environment variables instead.".to_string());
        result
            .messages
            .push("If this was accidental, the secret may need to be rotated.".to_string());
        result.messages.push(String::new());
    }

    if !analysis.todo_warnings.is_empty() {
        result.messages.push("## Untracked Work Items".to_string());
        result.messages.push(String::new());
        result.messages.push("Consider linking these items to beads issues:".to_string());
        result.messages.push(String::new());
        for w in &analysis.todo_warnings {
            result.messages.push(w.format());
        }
        result.messages.push(String::new());
    }
}

/// Check for interactive question handling.
fn check_interactive_question(
    transcript_info: &TranscriptInfo,
    sub_agent: &dyn SubAgent,
) -> Result<Option<StopHookResult>> {
    let Some(ref output) = transcript_info.last_assistant_output else {
        return Ok(None);
    };

    // Check if it looks like a question
    if !looks_like_question(output) {
        return Ok(None);
    }

    // Check if user is recently active
    if !transcript::is_user_recently_active(transcript_info, USER_RECENCY_MINUTES) {
        return Ok(None);
    }

    // Truncate for context
    let context = truncate_for_context(output, 2000);

    // Fast path: Auto-answer "should I continue?" questions
    if is_continue_question(context) {
        return Ok(Some(
            StopHookResult::block()
                .with_message("# Fast path: Auto-answering continue question")
                .with_inject("Yes, please continue."),
        ));
    }

    // Run sub-agent decision
    let decision = sub_agent.decide_on_question(context, USER_RECENCY_MINUTES)?;

    match decision {
        SubAgentDecision::AllowStop(reason) => {
            let mut result = StopHookResult::allow()
                .with_message("# Allowing stop for user interaction")
                .with_message("")
                .with_message("The agent appears to be asking a question and you were active.")
                .with_message("Please respond to continue the conversation.");
            if let Some(r) = reason {
                result.messages.insert(2, format!("Reason: {r}"));
            }
            Ok(Some(result))
        }
        SubAgentDecision::Answer(answer) => Ok(Some(
            StopHookResult::block()
                .with_message("# Sub-agent Response")
                .with_message("")
                .with_message(&answer)
                .with_message("")
                .with_message("---")
                .with_message("Continuing autonomous work...")
                .with_inject(answer),
        )),
        SubAgentDecision::Continue => Ok(None),
    }
}

/// Handle autonomous mode.
#[allow(clippy::too_many_lines)] // Complex logic with many status checks
fn handle_autonomous_mode(
    session: &mut SessionConfig,
    config: &StopHookConfig,
    runner: &dyn CommandRunner,
    _sub_agent: &dyn SubAgent,
) -> Result<StopHookResult> {
    let session_path = Path::new(session::SESSION_FILE_PATH);

    // Increment iteration
    session.iteration += 1;
    let iteration = session.iteration;

    // Get current issue state (if beads is available)
    let (open_ids, in_progress_ids) = if beads::is_beads_available(runner) {
        beads::get_current_issues(runner)?
    } else {
        (HashSet::new(), HashSet::new())
    };

    let current_snapshot: HashSet<String> = open_ids.union(&in_progress_ids).cloned().collect();
    let previous_snapshot = session.issue_snapshot_set();
    let total_outstanding = current_snapshot.len();

    // Check if issues changed
    if current_snapshot != previous_snapshot {
        session.last_issue_change_iteration = iteration;
    }

    // Update session file
    session.issue_snapshot = current_snapshot.into_iter().collect();
    session::write_session_file(session_path, session)?;

    // Check staleness
    let iterations_since_change = session.iterations_since_change();
    if iterations_since_change >= STALENESS_THRESHOLD {
        session::cleanup_session_file(session_path)?;
        return Ok(StopHookResult::allow()
            .with_message("# Staleness Detected")
            .with_message("")
            .with_message(format!("No issue changes for {iterations_since_change} iterations."))
            .with_message("Autonomous mode is stopping due to lack of progress.")
            .with_message("")
            .with_message("This could mean:")
            .with_message("- The remaining work requires human decisions")
            .with_message("- There's a blocker that needs manual intervention")
            .with_message("- The loop is stuck in an unproductive pattern")
            .with_message("")
            .with_message("Run `/autonomous-mode` to start a new session with fresh goals."));
    }

    // Check if all work is done
    if total_outstanding == 0 {
        let mut result = StopHookResult::block()
            .with_message("# Checking Completion")
            .with_message("")
            .with_message("No outstanding issues. Running quality gates...");

        // Run quality checks if enabled
        if config.quality_check_enabled {
            if let Some(ref cmd) = config.quality_check_command {
                let output = runner.run("sh", &["-c", cmd], None)?;
                if output.success() {
                    result.messages.push(String::new());
                    result.messages.push("All quality gates passed!".to_string());
                    result.messages.push("No open issues remain.".to_string());
                    result.messages.push(String::new());
                    result.messages.push("## Options".to_string());
                    result.messages.push(String::new());
                    result.messages.push("1. Run `/ideate` to generate new work items".to_string());
                    result.messages.push("2. Say an exit phrase to end the session".to_string());
                    result.messages.push(String::new());
                    result.messages.push("To exit when work is complete:".to_string());
                    result.messages.push(String::new());
                    result.messages.push(format!("  \"{HUMAN_INPUT_REQUIRED}\""));
                    return Ok(result);
                }
                result.messages.push(String::new());
                result.messages.push("## Quality Gates Failed".to_string());
                result.messages.push(truncate_output(&output.combined_output(), 50));
                result.messages.push(String::new());
                result.messages.push("Fix issues before completing.".to_string());
                return Ok(result);
            }
        }
    }

    // Work remains
    let mut result = StopHookResult::block()
        .with_message("# Autonomous Mode Active")
        .with_message("")
        .with_message(format!(
            "**Iteration {iteration}** | Outstanding issues: {total_outstanding}"
        ))
        .with_message(format!("Iterations since last issue change: {iterations_since_change}"))
        .with_message("")
        .with_message("## Current State")
        .with_message(format!("- Open issues: {}", open_ids.len()))
        .with_message(format!("- In progress: {}", in_progress_ids.len()))
        .with_message("")
        .with_message("## Action Required")
        .with_message("")
        .with_message("Continue working on outstanding issues:")
        .with_message("")
        .with_message("1. Run `bd ready` to see available work")
        .with_message("2. Pick an issue and work on it")
        .with_message("3. Run quality checks after completing work")
        .with_message("4. Close completed issues with `bd close <id>`");

    if iterations_since_change > 2 {
        result.messages.push(String::new());
        result
            .messages
            .push(format!("**Warning**: No issue changes for {iterations_since_change} loops."));
        result.messages.push(format!("Staleness threshold: {STALENESS_THRESHOLD}"));
    }

    result.messages.push(String::new());
    result.messages.push("---".to_string());
    result.messages.push(String::new());
    result
        .messages
        .push("If you cannot proceed without human input, use one of these phrases:".to_string());
    result.messages.push(String::new());
    result.messages.push("When all your work is done:".to_string());
    result.messages.push(format!("  \"{HUMAN_INPUT_REQUIRED}\""));
    result.messages.push(String::new());
    result.messages.push("When you've hit a problem you can't solve:".to_string());
    result.messages.push(format!("  \"{PROBLEM_NEEDS_USER}\""));

    Ok(result)
}

/// Truncate output to the last N lines.
fn truncate_output(output: &str, max_lines: usize) -> String {
    let lines: Vec<&str> = output.lines().collect();
    if lines.len() <= max_lines {
        return output.to_string();
    }

    let mut result = format!("... (showing last {} of {} lines)\n", max_lines, lines.len());
    for line in lines.iter().skip(lines.len() - max_lines) {
        result.push_str("  ");
        result.push_str(line);
        result.push('\n');
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stop_hook_result_allow() {
        let result = StopHookResult::allow();
        assert!(result.allow_stop);
        assert_eq!(result.exit_code, 0);
    }

    #[test]
    fn test_stop_hook_result_block() {
        let result = StopHookResult::block();
        assert!(!result.allow_stop);
        assert_eq!(result.exit_code, 2);
    }

    #[test]
    fn test_stop_hook_result_with_messages() {
        let result = StopHookResult::block().with_message("First").with_message("Second");
        assert_eq!(result.messages, vec!["First", "Second"]);
    }

    #[test]
    fn test_stop_hook_result_with_inject() {
        let result = StopHookResult::block().with_inject("Continue");
        assert_eq!(result.inject_response, Some("Continue".to_string()));
    }

    #[test]
    fn test_truncate_output_short() {
        let output = "line1\nline2\nline3";
        assert_eq!(truncate_output(output, 10), output);
    }

    #[test]
    fn test_truncate_output_long() {
        let output = (1..=100).map(|i| format!("line{i}")).collect::<Vec<_>>().join("\n");
        let truncated = truncate_output(&output, 10);
        assert!(truncated.contains("showing last 10 of 100 lines"));
        assert!(truncated.contains("line100"));
        assert!(!truncated.contains("line1\n")); // line1 should be truncated
    }

    #[test]
    fn test_human_input_required_constant() {
        assert!(HUMAN_INPUT_REQUIRED.contains("human input"));
    }

    #[test]
    fn test_problem_needs_user_constant() {
        assert!(PROBLEM_NEEDS_USER.contains("problem"));
        assert!(PROBLEM_NEEDS_USER.contains("user input"));
    }
}
