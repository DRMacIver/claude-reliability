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
use std::path::{Path, PathBuf};

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
    /// Base directory for beads checks (defaults to current directory).
    /// Used by tests to avoid changing global CWD.
    pub base_dir: Option<PathBuf>,
}

impl StopHookConfig {
    /// Get the base directory for file operations, defaulting to current directory.
    fn base_dir(&self) -> &Path {
        self.base_dir.as_deref().unwrap_or_else(|| Path::new("."))
    }

    /// Get the session file path (relative to `base_dir`).
    fn session_path(&self) -> PathBuf {
        self.base_dir().join(session::SESSION_FILE_PATH)
    }
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
    let session_path = config.session_path();
    let session_config = session::parse_session_file(&session_path)?;

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
            if has_complete_phrase
                && !has_problem_phrase
                && beads::is_beads_available_in(runner, config.base_dir())
            {
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
            session::cleanup_session_file(&session_path)?;
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
    let base_dir = config.base_dir();
    if beads::is_beads_available_in(runner, base_dir) {
        let beads_status = beads::check_beads_interaction_in(runner, base_dir)?;
        if !beads_status.has_interaction && !beads_status.already_warned {
            beads::mark_beads_warning_given_in(base_dir)?;
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
    let session_path = config.session_path();

    // Increment iteration
    session.iteration += 1;
    let iteration = session.iteration;

    // Get current issue state (if beads is available)
    let (open_ids, in_progress_ids) = if beads::is_beads_available_in(runner, config.base_dir()) {
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
    session::write_session_file(&session_path, session)?;

    // Check staleness
    let iterations_since_change = session.iterations_since_change();
    if iterations_since_change >= STALENESS_THRESHOLD {
        session::cleanup_session_file(&session_path)?;
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
    use crate::testing::{MockCommandRunner, MockSubAgent};
    use crate::traits::CommandOutput;

    fn mock_clean_git() -> MockCommandRunner {
        let mut runner = MockCommandRunner::new();
        // git diff --stat (no changes)
        runner.expect(
            "git",
            &["diff", "--stat"],
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() },
        );
        // git diff --cached --stat (no staged)
        runner.expect(
            "git",
            &["diff", "--cached", "--stat"],
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() },
        );
        // git ls-files --others --exclude-standard (no untracked)
        runner.expect(
            "git",
            &["ls-files", "--others", "--exclude-standard"],
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() },
        );
        // git rev-list --count @{upstream}..HEAD (not ahead)
        runner.expect(
            "git",
            &["rev-list", "--count", "@{upstream}..HEAD"],
            CommandOutput { exit_code: 0, stdout: "0\n".to_string(), stderr: String::new() },
        );
        runner
    }

    #[test]
    fn test_run_stop_hook_clean_repo_allows_exit() {
        let runner = mock_clean_git();
        let sub_agent = MockSubAgent::new();
        let input = crate::hooks::HookInput::default();
        let config = StopHookConfig::default();

        let result = run_stop_hook(&input, &config, &runner, &sub_agent).unwrap();
        assert!(result.allow_stop);
        assert_eq!(result.exit_code, 0);
    }

    #[test]
    fn test_run_stop_hook_repo_critique_mode_bypasses() {
        let runner = MockCommandRunner::new(); // No expectations - shouldn't be called
        let sub_agent = MockSubAgent::new();
        let input = crate::hooks::HookInput::default();
        let config = StopHookConfig { repo_critique_mode: true, ..Default::default() };

        let result = run_stop_hook(&input, &config, &runner, &sub_agent).unwrap();
        assert!(result.allow_stop);
    }

    fn mock_uncommitted_changes() -> MockCommandRunner {
        let mut runner = MockCommandRunner::new();
        let has_changes = CommandOutput {
            exit_code: 0,
            stdout: " file.rs | 10 ++++++++++\n".to_string(),
            stderr: String::new(),
        };
        let empty_success =
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() };
        let zero_commits =
            CommandOutput { exit_code: 0, stdout: "0\n".to_string(), stderr: String::new() };

        // First check_uncommitted_changes (fast path check)
        runner.expect("git", &["diff", "--stat"], has_changes.clone());
        runner.expect("git", &["diff", "--cached", "--stat"], empty_success.clone());
        runner.expect(
            "git",
            &["ls-files", "--others", "--exclude-standard"],
            empty_success.clone(),
        );
        runner.expect("git", &["rev-list", "--count", "@{upstream}..HEAD"], zero_commits.clone());

        // Second check_uncommitted_changes (main check)
        runner.expect("git", &["diff", "--stat"], has_changes);
        runner.expect("git", &["diff", "--cached", "--stat"], empty_success.clone());
        runner.expect(
            "git",
            &["ls-files", "--others", "--exclude-standard"],
            empty_success.clone(),
        );
        runner.expect("git", &["rev-list", "--count", "@{upstream}..HEAD"], zero_commits);

        // beads availability check uses runner.is_available(), not a command
        // Since we don't call runner.set_available("bd"), beads is not available

        // combined_diff for analysis (staged_diff first, then unstaged_diff)
        runner.expect("git", &["diff", "--cached", "-U0"], empty_success.clone());
        runner.expect("git", &["diff", "-U0"], empty_success);

        runner
    }

    #[test]
    fn test_run_stop_hook_uncommitted_changes_blocks() {
        let runner = mock_uncommitted_changes();
        let sub_agent = MockSubAgent::new();
        let input = crate::hooks::HookInput::default();
        let config = StopHookConfig::default();

        let result = run_stop_hook(&input, &config, &runner, &sub_agent).unwrap();
        assert!(!result.allow_stop);
        assert_eq!(result.exit_code, 2);
        assert!(result.messages.iter().any(|m| m.contains("Uncommitted Changes")));
    }

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

    #[test]
    fn test_stop_hook_result_with_messages_iter() {
        let msgs = vec!["A".to_string(), "B".to_string()];
        let result = StopHookResult::block().with_messages(msgs);
        assert_eq!(result.messages.len(), 2);
        assert_eq!(result.messages[0], "A");
        assert_eq!(result.messages[1], "B");
    }

    #[test]
    fn test_add_analysis_messages_suppression() {
        let mut result = StopHookResult::block();
        let analysis = AnalysisResults {
            suppression_violations: vec![crate::analysis::Violation::new(
                "test.py",
                1,
                "noqa violation",
            )],
            ..Default::default()
        };
        add_analysis_messages(&mut result, &analysis);
        assert!(result.messages.iter().any(|m| m.contains("Error Suppression")));
    }

    #[test]
    fn test_add_analysis_messages_empty_except() {
        let mut result = StopHookResult::block();
        let analysis = AnalysisResults {
            empty_except_violations: vec![crate::analysis::Violation::new(
                "test.py",
                1,
                "empty except",
            )],
            ..Default::default()
        };
        add_analysis_messages(&mut result, &analysis);
        assert!(result.messages.iter().any(|m| m.contains("Empty Exception")));
    }

    #[test]
    fn test_add_analysis_messages_secrets() {
        let mut result = StopHookResult::block();
        let analysis = AnalysisResults {
            secret_violations: vec![crate::analysis::Violation::new(
                "test.py",
                1,
                "hardcoded secret",
            )],
            ..Default::default()
        };
        add_analysis_messages(&mut result, &analysis);
        assert!(result.messages.iter().any(|m| m.contains("SECURITY")));
        assert!(result.messages.iter().any(|m| m.contains("Hardcoded Secrets")));
    }

    #[test]
    fn test_add_analysis_messages_todos() {
        let mut result = StopHookResult::block();
        let analysis = AnalysisResults {
            todo_warnings: vec![crate::analysis::Violation::new("test.py", 1, "TODO found")],
            ..Default::default()
        };
        add_analysis_messages(&mut result, &analysis);
        assert!(result.messages.iter().any(|m| m.contains("Untracked Work")));
    }

    fn mock_clean_with_ahead() -> MockCommandRunner {
        let mut runner = MockCommandRunner::new();
        let empty_success =
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() };
        let three_ahead =
            CommandOutput { exit_code: 0, stdout: "3\n".to_string(), stderr: String::new() };

        // First check_uncommitted_changes (fast path check)
        runner.expect("git", &["diff", "--stat"], empty_success.clone());
        runner.expect("git", &["diff", "--cached", "--stat"], empty_success.clone());
        runner.expect(
            "git",
            &["ls-files", "--others", "--exclude-standard"],
            empty_success.clone(),
        );
        runner.expect("git", &["rev-list", "--count", "@{upstream}..HEAD"], three_ahead.clone());

        // Fast path doesn't return because ahead_of_remote is true
        // Second check_uncommitted_changes (main check)
        runner.expect("git", &["diff", "--stat"], empty_success.clone());
        runner.expect("git", &["diff", "--cached", "--stat"], empty_success.clone());
        runner.expect("git", &["ls-files", "--others", "--exclude-standard"], empty_success);
        runner.expect("git", &["rev-list", "--count", "@{upstream}..HEAD"], three_ahead);

        runner
    }

    #[test]
    fn test_run_stop_hook_unpushed_commits_blocks() {
        let runner = mock_clean_with_ahead();
        let sub_agent = MockSubAgent::new();
        let input = crate::hooks::HookInput::default();
        let config = StopHookConfig { require_push: true, ..Default::default() };

        let result = run_stop_hook(&input, &config, &runner, &sub_agent).unwrap();
        assert!(!result.allow_stop);
        assert_eq!(result.exit_code, 2);
        assert!(result.messages.iter().any(|m| m.contains("Unpushed Commits")));
    }

    #[test]
    fn test_run_stop_hook_unpushed_allowed_without_require_push() {
        let runner = mock_clean_with_ahead();
        let sub_agent = MockSubAgent::new();
        let input = crate::hooks::HookInput::default();
        let config = StopHookConfig { require_push: false, ..Default::default() };

        let result = run_stop_hook(&input, &config, &runner, &sub_agent).unwrap();
        // Without require_push, being ahead is allowed
        assert!(result.allow_stop);
    }

    fn mock_uncommitted_with_untracked() -> MockCommandRunner {
        let mut runner = MockCommandRunner::new();
        let has_changes = CommandOutput {
            exit_code: 0,
            stdout: " file.rs | 10 ++++++++++\n".to_string(),
            stderr: String::new(),
        };
        let empty_success =
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() };
        let zero_commits =
            CommandOutput { exit_code: 0, stdout: "0\n".to_string(), stderr: String::new() };
        let untracked_files = CommandOutput {
            exit_code: 0,
            stdout: "untracked1.txt\nuntracked2.txt\n".to_string(),
            stderr: String::new(),
        };

        // First check_uncommitted_changes (fast path check)
        runner.expect("git", &["diff", "--stat"], has_changes.clone());
        runner.expect("git", &["diff", "--cached", "--stat"], empty_success.clone());
        runner.expect(
            "git",
            &["ls-files", "--others", "--exclude-standard"],
            untracked_files.clone(),
        );
        runner.expect("git", &["rev-list", "--count", "@{upstream}..HEAD"], zero_commits.clone());

        // Second check_uncommitted_changes (main check)
        runner.expect("git", &["diff", "--stat"], has_changes);
        runner.expect("git", &["diff", "--cached", "--stat"], empty_success.clone());
        runner.expect("git", &["ls-files", "--others", "--exclude-standard"], untracked_files);
        runner.expect("git", &["rev-list", "--count", "@{upstream}..HEAD"], zero_commits);

        // combined_diff for analysis
        runner.expect("git", &["diff", "--cached", "-U0"], empty_success.clone());
        runner.expect("git", &["diff", "-U0"], empty_success);

        runner
    }

    #[test]
    fn test_run_stop_hook_shows_untracked_files() {
        let runner = mock_uncommitted_with_untracked();
        let sub_agent = MockSubAgent::new();
        let input = crate::hooks::HookInput::default();
        let config = StopHookConfig::default();

        let result = run_stop_hook(&input, &config, &runner, &sub_agent).unwrap();
        assert!(!result.allow_stop);
        assert!(result.messages.iter().any(|m| m.contains("Untracked Files")));
        assert!(result.messages.iter().any(|m| m.contains("untracked1.txt")));
    }

    fn mock_uncommitted_with_quality_checks() -> MockCommandRunner {
        let mut runner = MockCommandRunner::new();
        let has_changes = CommandOutput {
            exit_code: 0,
            stdout: " file.rs | 10 ++++++++++\n".to_string(),
            stderr: String::new(),
        };
        let empty_success =
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() };
        let zero_commits =
            CommandOutput { exit_code: 0, stdout: "0\n".to_string(), stderr: String::new() };

        // First check_uncommitted_changes (fast path check)
        runner.expect("git", &["diff", "--stat"], has_changes.clone());
        runner.expect("git", &["diff", "--cached", "--stat"], empty_success.clone());
        runner.expect(
            "git",
            &["ls-files", "--others", "--exclude-standard"],
            empty_success.clone(),
        );
        runner.expect("git", &["rev-list", "--count", "@{upstream}..HEAD"], zero_commits.clone());

        // Second check_uncommitted_changes (main check)
        runner.expect("git", &["diff", "--stat"], has_changes);
        runner.expect("git", &["diff", "--cached", "--stat"], empty_success.clone());
        runner.expect(
            "git",
            &["ls-files", "--others", "--exclude-standard"],
            empty_success.clone(),
        );
        runner.expect("git", &["rev-list", "--count", "@{upstream}..HEAD"], zero_commits);

        // combined_diff for analysis
        runner.expect("git", &["diff", "--cached", "-U0"], empty_success.clone());
        runner.expect("git", &["diff", "-U0"], empty_success);

        // Quality check command
        runner.expect(
            "sh",
            &["-c", "just check"],
            CommandOutput {
                exit_code: 1,
                stdout: "Error: lint failed".to_string(),
                stderr: String::new(),
            },
        );

        runner
    }

    #[test]
    fn test_run_stop_hook_quality_check_fails() {
        let runner = mock_uncommitted_with_quality_checks();
        let sub_agent = MockSubAgent::new();
        let input = crate::hooks::HookInput::default();
        let config = StopHookConfig {
            quality_check_enabled: true,
            quality_check_command: Some("just check".to_string()),
            ..Default::default()
        };

        let result = run_stop_hook(&input, &config, &runner, &sub_agent).unwrap();
        assert!(!result.allow_stop);
        assert!(result.messages.iter().any(|m| m.contains("Quality Checks Failed")));
    }

    #[test]
    fn test_check_interactive_question_no_output() {
        let transcript_info = TranscriptInfo::default();
        let sub_agent = MockSubAgent::new();

        let result = check_interactive_question(&transcript_info, &sub_agent).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_check_interactive_question_not_a_question() {
        let transcript_info = TranscriptInfo {
            last_assistant_output: Some("This is just a statement.".to_string()),
            last_user_message_time: None,
        };
        let sub_agent = MockSubAgent::new();

        let result = check_interactive_question(&transcript_info, &sub_agent).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_check_interactive_question_user_not_active() {
        use chrono::{Duration, Utc};
        let transcript_info = TranscriptInfo {
            last_assistant_output: Some("Would you like me to continue?".to_string()),
            // User was active 10 minutes ago (beyond the 5-minute threshold)
            last_user_message_time: Some(Utc::now() - Duration::minutes(10)),
        };
        let sub_agent = MockSubAgent::new();

        let result = check_interactive_question(&transcript_info, &sub_agent).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_check_interactive_question_continue_question() {
        use chrono::{Duration, Utc};
        let transcript_info = TranscriptInfo {
            last_assistant_output: Some("Should I continue?".to_string()),
            last_user_message_time: Some(Utc::now() - Duration::minutes(1)),
        };
        let sub_agent = MockSubAgent::new();

        let result = check_interactive_question(&transcript_info, &sub_agent).unwrap();
        assert!(result.is_some());
        let result = result.unwrap();
        assert!(!result.allow_stop);
        assert!(result.inject_response.is_some());
        assert!(result.inject_response.unwrap().contains("continue"));
    }

    #[test]
    fn test_check_interactive_question_subagent_allow_stop() {
        use crate::traits::SubAgentDecision;
        use chrono::{Duration, Utc};

        let transcript_info = TranscriptInfo {
            last_assistant_output: Some("What color theme would you prefer?".to_string()),
            last_user_message_time: Some(Utc::now() - Duration::minutes(1)),
        };
        let mut sub_agent = MockSubAgent::new();
        sub_agent.expect_question_decision(SubAgentDecision::AllowStop(Some(
            "User preference needed".to_string(),
        )));

        let result = check_interactive_question(&transcript_info, &sub_agent).unwrap();
        assert!(result.is_some());
        let result = result.unwrap();
        assert!(result.allow_stop);
        assert!(result.messages.iter().any(|m| m.contains("user interaction")));
    }

    #[test]
    fn test_check_interactive_question_subagent_answer() {
        use crate::traits::SubAgentDecision;
        use chrono::{Duration, Utc};

        let transcript_info = TranscriptInfo {
            last_assistant_output: Some("Which approach should I use?".to_string()),
            last_user_message_time: Some(Utc::now() - Duration::minutes(1)),
        };
        let mut sub_agent = MockSubAgent::new();
        sub_agent.expect_question_decision(SubAgentDecision::Answer("Use approach A".to_string()));

        let result = check_interactive_question(&transcript_info, &sub_agent).unwrap();
        assert!(result.is_some());
        let result = result.unwrap();
        assert!(!result.allow_stop);
        assert_eq!(result.inject_response, Some("Use approach A".to_string()));
    }

    #[test]
    fn test_check_interactive_question_subagent_continue() {
        use crate::traits::SubAgentDecision;
        use chrono::{Duration, Utc};

        let transcript_info = TranscriptInfo {
            last_assistant_output: Some("What do you think about this?".to_string()),
            last_user_message_time: Some(Utc::now() - Duration::minutes(1)),
        };
        let mut sub_agent = MockSubAgent::new();
        sub_agent.expect_question_decision(SubAgentDecision::Continue);

        let result = check_interactive_question(&transcript_info, &sub_agent).unwrap();
        assert!(result.is_none());
    }

    fn create_transcript_with_output(output: &str) -> tempfile::NamedTempFile {
        use std::io::Write;
        let mut file = tempfile::NamedTempFile::new().unwrap();
        let entry = serde_json::json!({
            "type": "assistant",
            "message": {
                "content": [
                    {"type": "text", "text": output}
                ]
            }
        });
        writeln!(file, "{}", serde_json::to_string(&entry).unwrap()).unwrap();
        file
    }

    fn mock_with_uncommitted_for_bypass() -> MockCommandRunner {
        let mut runner = MockCommandRunner::new();
        let has_changes = CommandOutput {
            exit_code: 0,
            stdout: " file.rs | 10 ++++++++++\n".to_string(),
            stderr: String::new(),
        };
        let empty_success =
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() };
        let zero_commits =
            CommandOutput { exit_code: 0, stdout: "0\n".to_string(), stderr: String::new() };

        // Fast path check - returns changes so we don't exit early
        runner.expect("git", &["diff", "--stat"], has_changes);
        runner.expect("git", &["diff", "--cached", "--stat"], empty_success.clone());
        runner.expect("git", &["ls-files", "--others", "--exclude-standard"], empty_success);
        runner.expect("git", &["rev-list", "--count", "@{upstream}..HEAD"], zero_commits);

        runner
    }

    #[test]
    fn test_bypass_problem_phrase_allows_exit() {
        let transcript_file = create_transcript_with_output(PROBLEM_NEEDS_USER);
        let runner = mock_with_uncommitted_for_bypass();
        let sub_agent = MockSubAgent::new();
        let input = crate::hooks::HookInput {
            transcript_path: Some(transcript_file.path().to_string_lossy().to_string()),
            ..Default::default()
        };
        let config = StopHookConfig::default();

        let result = run_stop_hook(&input, &config, &runner, &sub_agent).unwrap();
        assert!(result.allow_stop);
        assert!(result.messages.iter().any(|m| m.contains("Problem requiring user input")));
    }

    #[test]
    fn test_bypass_human_input_phrase_allows_exit() {
        let transcript_file = create_transcript_with_output(HUMAN_INPUT_REQUIRED);
        let runner = mock_with_uncommitted_for_bypass();
        let sub_agent = MockSubAgent::new();
        let input = crate::hooks::HookInput {
            transcript_path: Some(transcript_file.path().to_string_lossy().to_string()),
            ..Default::default()
        };
        let config = StopHookConfig::default();

        let result = run_stop_hook(&input, &config, &runner, &sub_agent).unwrap();
        assert!(result.allow_stop);
        assert!(result.messages.iter().any(|m| m.contains("Human input required")));
    }

    #[test]
    fn test_bypass_human_input_blocked_with_open_issues() {
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // Create .beads directory
        std::fs::create_dir_all(base.join(".beads")).unwrap();

        let transcript_file = create_transcript_with_output(HUMAN_INPUT_REQUIRED);

        let mut runner = MockCommandRunner::new();
        runner.set_available("bd");

        let has_changes = CommandOutput {
            exit_code: 0,
            stdout: " file.rs | 10 ++++++++++\n".to_string(),
            stderr: String::new(),
        };
        let empty_success =
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() };
        let zero_commits =
            CommandOutput { exit_code: 0, stdout: "0\n".to_string(), stderr: String::new() };

        // check_uncommitted_changes (fast path) - returns changes so we continue
        runner.expect("git", &["diff", "--stat"], has_changes);
        runner.expect("git", &["diff", "--cached", "--stat"], empty_success.clone());
        runner.expect("git", &["ls-files", "--others", "--exclude-standard"], empty_success);
        runner.expect("git", &["rev-list", "--count", "@{upstream}..HEAD"], zero_commits);

        // beads availability check happens in bypass check
        // get_open_issues_count - returns 2 open issues
        runner.expect(
            "bd",
            &["list", "--status=open", "--format=json"],
            CommandOutput {
                exit_code: 0,
                stdout: r#"[{"id": "a"}, {"id": "b"}]"#.to_string(),
                stderr: String::new(),
            },
        );

        let sub_agent = MockSubAgent::new();
        let input = crate::hooks::HookInput {
            transcript_path: Some(transcript_file.path().to_string_lossy().to_string()),
            ..Default::default()
        };
        let config = StopHookConfig { base_dir: Some(base.to_path_buf()), ..Default::default() };

        let result = run_stop_hook(&input, &config, &runner, &sub_agent).unwrap();
        assert!(!result.allow_stop);
        assert!(result.messages.iter().any(|m| m.contains("Exit Phrase Rejected")));
        assert!(result.messages.iter().any(|m| m.contains("2 open issue")));
    }

    #[test]
    fn test_beads_warning_on_uncommitted_changes() {
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // Create .beads directory so beads is "available"
        std::fs::create_dir_all(base.join(".beads")).unwrap();

        let mut runner = MockCommandRunner::new();
        runner.set_available("bd");

        let has_changes = CommandOutput {
            exit_code: 0,
            stdout: " file.rs | 10 ++++++++++\n".to_string(),
            stderr: String::new(),
        };
        let empty_success =
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() };
        let zero_commits =
            CommandOutput { exit_code: 0, stdout: "0\n".to_string(), stderr: String::new() };

        // First check_uncommitted_changes (fast path)
        runner.expect("git", &["diff", "--stat"], has_changes.clone());
        runner.expect("git", &["diff", "--cached", "--stat"], empty_success.clone());
        runner.expect(
            "git",
            &["ls-files", "--others", "--exclude-standard"],
            empty_success.clone(),
        );
        runner.expect("git", &["rev-list", "--count", "@{upstream}..HEAD"], zero_commits.clone());

        // Second check_uncommitted_changes (main check)
        runner.expect("git", &["diff", "--stat"], has_changes);
        runner.expect("git", &["diff", "--cached", "--stat"], empty_success.clone());
        runner.expect("git", &["ls-files", "--others", "--exclude-standard"], empty_success);
        runner.expect("git", &["rev-list", "--count", "@{upstream}..HEAD"], zero_commits);

        // check_beads_interaction - no interaction
        runner.expect(
            "git",
            &["diff", "--name-only", "--", ".beads/"],
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() },
        );
        runner.expect(
            "git",
            &["diff", "--cached", "--name-only", "--", ".beads/"],
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() },
        );
        runner.expect(
            "git",
            &["log", "--oneline", "-10", "--name-only", "--", ".beads/"],
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() },
        );

        let sub_agent = MockSubAgent::new();
        let input = crate::hooks::HookInput::default();
        let config = StopHookConfig { base_dir: Some(base.to_path_buf()), ..Default::default() };

        let result = run_stop_hook(&input, &config, &runner, &sub_agent).unwrap();
        assert!(!result.allow_stop);
        assert!(result.messages.iter().any(|m| m.contains("Beads Interaction Required")));
    }

    #[test]
    fn test_quality_check_passes() {
        let mut runner = MockCommandRunner::new();

        let empty_success =
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() };
        let zero_commits =
            CommandOutput { exit_code: 0, stdout: "0\n".to_string(), stderr: String::new() };

        // check_uncommitted_changes (fast path) - clean
        runner.expect("git", &["diff", "--stat"], empty_success.clone());
        runner.expect("git", &["diff", "--cached", "--stat"], empty_success.clone());
        runner.expect("git", &["ls-files", "--others", "--exclude-standard"], empty_success);
        runner.expect("git", &["rev-list", "--count", "@{upstream}..HEAD"], zero_commits);

        let sub_agent = MockSubAgent::new();
        let input = crate::hooks::HookInput::default();
        let config = StopHookConfig {
            quality_check_enabled: true,
            quality_check_command: Some("just check".to_string()),
            ..Default::default()
        };

        // Clean repo, so fast path returns early without running quality checks
        let result = run_stop_hook(&input, &config, &runner, &sub_agent).unwrap();
        assert!(result.allow_stop);
    }

    #[test]
    fn test_quality_check_fails_not_autonomous() {
        // Test quality check failure when NOT in autonomous mode
        // This requires: no session, ahead of remote (skip fast path),
        // require_push=false, quality check enabled and fails
        let mut runner = MockCommandRunner::new();

        let empty_success =
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() };
        // Ahead of remote by 1 commit (skips fast path)
        let one_commit =
            CommandOutput { exit_code: 0, stdout: "1\n".to_string(), stderr: String::new() };
        let quality_fail = CommandOutput {
            exit_code: 1,
            stdout: String::new(),
            stderr: "lint failed\n".to_string(),
        };

        // check_uncommitted_changes (fast path check) - no uncommitted but ahead
        runner.expect("git", &["diff", "--stat"], empty_success.clone());
        runner.expect("git", &["diff", "--cached", "--stat"], empty_success.clone());
        runner.expect(
            "git",
            &["ls-files", "--others", "--exclude-standard"],
            empty_success.clone(),
        );
        runner.expect("git", &["rev-list", "--count", "@{upstream}..HEAD"], one_commit.clone());

        // Second check_uncommitted_changes
        runner.expect("git", &["diff", "--stat"], empty_success.clone());
        runner.expect("git", &["diff", "--cached", "--stat"], empty_success.clone());
        runner.expect("git", &["ls-files", "--others", "--exclude-standard"], empty_success);
        runner.expect("git", &["rev-list", "--count", "@{upstream}..HEAD"], one_commit);

        // Quality check fails
        runner.expect("sh", &["-c", "just check"], quality_fail);

        let sub_agent = MockSubAgent::new();
        let input = crate::hooks::HookInput::default();
        let config = StopHookConfig {
            quality_check_enabled: true,
            quality_check_command: Some("just check".to_string()),
            require_push: false, // Don't block on unpushed commits
            ..Default::default()
        };

        let result = run_stop_hook(&input, &config, &runner, &sub_agent).unwrap();
        assert!(!result.allow_stop);
        assert!(result.messages.iter().any(|m| m.contains("Quality Gates Failed")));
    }

    // Helper to create a session file for autonomous mode tests
    fn create_session_file(base: &std::path::Path, iteration: u32, last_change: u32) {
        let session_dir = base.join(".claude");
        std::fs::create_dir_all(&session_dir).unwrap();
        let content = format!(
            "---\niteration: {iteration}\nlast_issue_change_iteration: {last_change}\nissue_snapshot: []\n---\n# Session"
        );
        std::fs::write(session_dir.join("autonomous-session.local.md"), content).unwrap();
    }

    #[test]
    fn test_autonomous_mode_work_remaining() {
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // Create session file (iteration 3, last change at 2)
        create_session_file(base, 3, 2);

        let mut runner = MockCommandRunner::new();
        runner.set_available("bd");

        let empty_success =
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() };
        let zero_commits =
            CommandOutput { exit_code: 0, stdout: "0\n".to_string(), stderr: String::new() };

        // check_uncommitted_changes (fast path skipped due to session file)
        runner.expect("git", &["diff", "--stat"], empty_success.clone());
        runner.expect("git", &["diff", "--cached", "--stat"], empty_success.clone());
        runner.expect("git", &["ls-files", "--others", "--exclude-standard"], empty_success);
        runner.expect("git", &["rev-list", "--count", "@{upstream}..HEAD"], zero_commits);

        // get_current_issues (beads not available since no .beads dir)
        // So no beads commands expected

        let sub_agent = MockSubAgent::new();
        let input = crate::hooks::HookInput::default();
        let config = StopHookConfig { base_dir: Some(base.to_path_buf()), ..Default::default() };

        let result = run_stop_hook(&input, &config, &runner, &sub_agent).unwrap();
        assert!(!result.allow_stop);
        assert!(result.messages.iter().any(|m| m.contains("Autonomous Mode Active")));
        assert!(result.messages.iter().any(|m| m.contains("Iteration 4")));
    }

    #[test]
    fn test_autonomous_mode_staleness_detected() {
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // Create session file with high staleness (iteration 10, last change at 3)
        // This means iterations_since_change = 10-3 = 7, and after increment = 11-3 = 8 >= 5
        create_session_file(base, 10, 3);

        let mut runner = MockCommandRunner::new();

        let empty_success =
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() };
        let zero_commits =
            CommandOutput { exit_code: 0, stdout: "0\n".to_string(), stderr: String::new() };

        // check_uncommitted_changes (fast path skipped)
        runner.expect("git", &["diff", "--stat"], empty_success.clone());
        runner.expect("git", &["diff", "--cached", "--stat"], empty_success.clone());
        runner.expect("git", &["ls-files", "--others", "--exclude-standard"], empty_success);
        runner.expect("git", &["rev-list", "--count", "@{upstream}..HEAD"], zero_commits);

        let sub_agent = MockSubAgent::new();
        let input = crate::hooks::HookInput::default();
        let config = StopHookConfig { base_dir: Some(base.to_path_buf()), ..Default::default() };

        let result = run_stop_hook(&input, &config, &runner, &sub_agent).unwrap();
        assert!(result.allow_stop);
        assert!(result.messages.iter().any(|m| m.contains("Staleness Detected")));
    }

    #[test]
    fn test_autonomous_mode_all_done_quality_passes() {
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // Create session file (iteration 1, last change at 1)
        create_session_file(base, 1, 1);

        let mut runner = MockCommandRunner::new();

        let empty_success =
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() };
        let zero_commits =
            CommandOutput { exit_code: 0, stdout: "0\n".to_string(), stderr: String::new() };

        // check_uncommitted_changes (fast path skipped)
        runner.expect("git", &["diff", "--stat"], empty_success.clone());
        runner.expect("git", &["diff", "--cached", "--stat"], empty_success.clone());
        runner.expect(
            "git",
            &["ls-files", "--others", "--exclude-standard"],
            empty_success.clone(),
        );
        runner.expect("git", &["rev-list", "--count", "@{upstream}..HEAD"], zero_commits);

        // Quality check passes
        runner.expect("sh", &["-c", "just check"], empty_success);

        let sub_agent = MockSubAgent::new();
        let input = crate::hooks::HookInput::default();
        let config = StopHookConfig {
            quality_check_enabled: true,
            quality_check_command: Some("just check".to_string()),
            base_dir: Some(base.to_path_buf()),
            ..Default::default()
        };

        let result = run_stop_hook(&input, &config, &runner, &sub_agent).unwrap();
        assert!(!result.allow_stop);
        assert!(result.messages.iter().any(|m| m.contains("Checking Completion")));
        assert!(result.messages.iter().any(|m| m.contains("All quality gates passed")));
    }

    #[test]
    fn test_autonomous_mode_all_done_quality_fails() {
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // Create session file (iteration 1, last change at 1)
        create_session_file(base, 1, 1);

        let mut runner = MockCommandRunner::new();

        let empty_success =
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() };
        let zero_commits =
            CommandOutput { exit_code: 0, stdout: "0\n".to_string(), stderr: String::new() };
        let quality_fail = CommandOutput {
            exit_code: 1,
            stdout: String::new(),
            stderr: "Test failed\n".to_string(),
        };

        // check_uncommitted_changes (fast path skipped)
        runner.expect("git", &["diff", "--stat"], empty_success.clone());
        runner.expect("git", &["diff", "--cached", "--stat"], empty_success.clone());
        runner.expect("git", &["ls-files", "--others", "--exclude-standard"], empty_success);
        runner.expect("git", &["rev-list", "--count", "@{upstream}..HEAD"], zero_commits);

        // Quality check fails
        runner.expect("sh", &["-c", "just check"], quality_fail);

        let sub_agent = MockSubAgent::new();
        let input = crate::hooks::HookInput::default();
        let config = StopHookConfig {
            quality_check_enabled: true,
            quality_check_command: Some("just check".to_string()),
            base_dir: Some(base.to_path_buf()),
            ..Default::default()
        };

        let result = run_stop_hook(&input, &config, &runner, &sub_agent).unwrap();
        assert!(!result.allow_stop);
        assert!(result.messages.iter().any(|m| m.contains("Quality Gates Failed")));
    }

    #[test]
    fn test_autonomous_mode_with_staleness_warning() {
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // Create session file (iteration 5, last change at 2 - gives 3 iterations since change)
        // After increment it becomes iteration 6, so iterations_since_change = 4 > 2
        create_session_file(base, 5, 2);

        let mut runner = MockCommandRunner::new();

        let empty_success =
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() };
        let zero_commits =
            CommandOutput { exit_code: 0, stdout: "0\n".to_string(), stderr: String::new() };

        // check_uncommitted_changes (fast path skipped)
        runner.expect("git", &["diff", "--stat"], empty_success.clone());
        runner.expect("git", &["diff", "--cached", "--stat"], empty_success.clone());
        runner.expect("git", &["ls-files", "--others", "--exclude-standard"], empty_success);
        runner.expect("git", &["rev-list", "--count", "@{upstream}..HEAD"], zero_commits);

        let sub_agent = MockSubAgent::new();
        let input = crate::hooks::HookInput::default();
        let config = StopHookConfig { base_dir: Some(base.to_path_buf()), ..Default::default() };

        let result = run_stop_hook(&input, &config, &runner, &sub_agent).unwrap();
        assert!(!result.allow_stop);
        assert!(result.messages.iter().any(|m| m.contains("Autonomous Mode Active")));
        // Check for staleness warning (iterations > 2)
        assert!(result.messages.iter().any(|m| m.contains("Warning")));
    }

    #[test]
    fn test_autonomous_mode_with_beads_issues() {
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // Create session file and .beads directory
        create_session_file(base, 1, 1);
        std::fs::create_dir_all(base.join(".beads")).unwrap();

        let mut runner = MockCommandRunner::new();
        runner.set_available("bd");

        let empty_success =
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() };
        let zero_commits =
            CommandOutput { exit_code: 0, stdout: "0\n".to_string(), stderr: String::new() };

        // check_uncommitted_changes (fast path skipped)
        runner.expect("git", &["diff", "--stat"], empty_success.clone());
        runner.expect("git", &["diff", "--cached", "--stat"], empty_success.clone());
        runner.expect("git", &["ls-files", "--others", "--exclude-standard"], empty_success);
        runner.expect("git", &["rev-list", "--count", "@{upstream}..HEAD"], zero_commits);

        // get_current_issues - returns some issues
        runner.expect(
            "bd",
            &["list", "--status=open"],
            CommandOutput {
                exit_code: 0,
                stdout: "proj-1 [P1] Test issue\n".to_string(),
                stderr: String::new(),
            },
        );
        runner.expect(
            "bd",
            &["list", "--status=in_progress"],
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() },
        );

        let sub_agent = MockSubAgent::new();
        let input = crate::hooks::HookInput::default();
        let config = StopHookConfig { base_dir: Some(base.to_path_buf()), ..Default::default() };

        let result = run_stop_hook(&input, &config, &runner, &sub_agent).unwrap();
        assert!(!result.allow_stop);
        assert!(result.messages.iter().any(|m| m.contains("Autonomous Mode Active")));
        assert!(result.messages.iter().any(|m| m.contains("Outstanding issues: 1")));
        assert!(result.messages.iter().any(|m| m.contains("Open issues: 1")));
    }

    #[test]
    fn test_run_stop_hook_shows_many_untracked_files() {
        // Test with more than 10 untracked files to cover the "... and X more" message
        let mut runner = MockCommandRunner::new();
        let has_changes = CommandOutput {
            exit_code: 0,
            stdout: " file.rs | 10 ++++++++++\n".to_string(),
            stderr: String::new(),
        };
        let empty_success =
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() };
        let zero_commits =
            CommandOutput { exit_code: 0, stdout: "0\n".to_string(), stderr: String::new() };

        // 15 untracked files
        let many_untracked = CommandOutput {
            exit_code: 0,
            stdout: "file1.txt\nfile2.txt\nfile3.txt\nfile4.txt\nfile5.txt\nfile6.txt\nfile7.txt\nfile8.txt\nfile9.txt\nfile10.txt\nfile11.txt\nfile12.txt\nfile13.txt\nfile14.txt\nfile15.txt\n".to_string(),
            stderr: String::new(),
        };

        // First check_uncommitted_changes (fast path check)
        runner.expect("git", &["diff", "--stat"], has_changes.clone());
        runner.expect("git", &["diff", "--cached", "--stat"], empty_success.clone());
        runner.expect(
            "git",
            &["ls-files", "--others", "--exclude-standard"],
            many_untracked.clone(),
        );
        runner.expect("git", &["rev-list", "--count", "@{upstream}..HEAD"], zero_commits.clone());

        // Second check_uncommitted_changes (main check)
        runner.expect("git", &["diff", "--stat"], has_changes);
        runner.expect("git", &["diff", "--cached", "--stat"], empty_success.clone());
        runner.expect("git", &["ls-files", "--others", "--exclude-standard"], many_untracked);
        runner.expect("git", &["rev-list", "--count", "@{upstream}..HEAD"], zero_commits);

        // combined_diff for analysis
        runner.expect("git", &["diff", "--cached", "-U0"], empty_success.clone());
        runner.expect("git", &["diff", "-U0"], empty_success);

        let sub_agent = MockSubAgent::new();
        let input = crate::hooks::HookInput::default();
        let config = StopHookConfig::default();

        let result = run_stop_hook(&input, &config, &runner, &sub_agent).unwrap();
        assert!(!result.allow_stop);
        // Should show "... and 5 more" for the extra files beyond 10
        assert!(result.messages.iter().any(|m| m.contains("... and 5 more")));
    }

    #[test]
    fn test_run_stop_hook_require_push_message() {
        // Test that require_push adds the push message
        let mut runner = MockCommandRunner::new();
        let has_changes = CommandOutput {
            exit_code: 0,
            stdout: " file.rs | 10 ++++++++++\n".to_string(),
            stderr: String::new(),
        };
        let empty_success =
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() };
        let zero_commits =
            CommandOutput { exit_code: 0, stdout: "0\n".to_string(), stderr: String::new() };

        // First check_uncommitted_changes (fast path check)
        runner.expect("git", &["diff", "--stat"], has_changes.clone());
        runner.expect("git", &["diff", "--cached", "--stat"], empty_success.clone());
        runner.expect(
            "git",
            &["ls-files", "--others", "--exclude-standard"],
            empty_success.clone(),
        );
        runner.expect("git", &["rev-list", "--count", "@{upstream}..HEAD"], zero_commits.clone());

        // Second check_uncommitted_changes (main check)
        runner.expect("git", &["diff", "--stat"], has_changes);
        runner.expect("git", &["diff", "--cached", "--stat"], empty_success.clone());
        runner.expect(
            "git",
            &["ls-files", "--others", "--exclude-standard"],
            empty_success.clone(),
        );
        runner.expect("git", &["rev-list", "--count", "@{upstream}..HEAD"], zero_commits);

        // combined_diff for analysis
        runner.expect("git", &["diff", "--cached", "-U0"], empty_success.clone());
        runner.expect("git", &["diff", "-U0"], empty_success);

        let sub_agent = MockSubAgent::new();
        let input = crate::hooks::HookInput::default();
        let config = StopHookConfig { require_push: true, ..Default::default() };

        let result = run_stop_hook(&input, &config, &runner, &sub_agent).unwrap();
        assert!(!result.allow_stop);
        // Should include push instruction when require_push is enabled
        assert!(result.messages.iter().any(|m| m.contains("Push to remote")));
        assert!(result.messages.iter().any(|m| m.contains("Work is incomplete until")));
    }

    #[test]
    fn test_run_stop_hook_interactive_question_allows_stop() {
        // Test interactive question path through run_stop_hook (covers line 209)
        // To reach check_interactive_question, we need:
        // 1. Skip fast path: ahead_of_remote = true OR uncommitted changes
        // 2. No uncommitted changes (to not go to handle_uncommitted_changes)
        // 3. require_push = false (to not block on unpushed commits)
        use chrono::{Duration, Utc};
        use std::io::Write;

        // Create transcript with a question and recent user activity
        let mut transcript_file = tempfile::NamedTempFile::new().unwrap();
        let user_time = Utc::now() - Duration::minutes(1);
        let user_entry = serde_json::json!({
            "type": "user",
            "timestamp": user_time.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
        });
        let assistant_entry = serde_json::json!({
            "type": "assistant",
            "message": {
                "content": [
                    {"type": "text", "text": "What color theme would you prefer?"}
                ]
            }
        });
        writeln!(transcript_file, "{}", serde_json::to_string(&user_entry).unwrap()).unwrap();
        writeln!(transcript_file, "{}", serde_json::to_string(&assistant_entry).unwrap()).unwrap();

        let mut runner = MockCommandRunner::new();
        let empty_success =
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() };
        // Ahead of remote by 1 commit (to skip fast path)
        let one_commit =
            CommandOutput { exit_code: 0, stdout: "1\n".to_string(), stderr: String::new() };

        // check_uncommitted_changes (fast path) - clean repo but ahead
        runner.expect("git", &["diff", "--stat"], empty_success.clone());
        runner.expect("git", &["diff", "--cached", "--stat"], empty_success.clone());
        runner.expect(
            "git",
            &["ls-files", "--others", "--exclude-standard"],
            empty_success.clone(),
        );
        runner.expect("git", &["rev-list", "--count", "@{upstream}..HEAD"], one_commit.clone());

        // Second check_uncommitted_changes (main check)
        runner.expect("git", &["diff", "--stat"], empty_success.clone());
        runner.expect("git", &["diff", "--cached", "--stat"], empty_success.clone());
        runner.expect("git", &["ls-files", "--others", "--exclude-standard"], empty_success);
        runner.expect("git", &["rev-list", "--count", "@{upstream}..HEAD"], one_commit);

        let mut sub_agent = MockSubAgent::new();
        sub_agent.expect_question_decision(SubAgentDecision::AllowStop(Some(
            "User preference needed".to_string(),
        )));

        let input = crate::hooks::HookInput {
            transcript_path: Some(transcript_file.path().to_string_lossy().to_string()),
            ..Default::default()
        };
        // require_push = false so we don't block on unpushed commits
        let config = StopHookConfig { require_push: false, ..Default::default() };

        let result = run_stop_hook(&input, &config, &runner, &sub_agent).unwrap();
        assert!(result.allow_stop);
        assert!(result.messages.iter().any(|m| m.contains("user interaction")));
    }

    #[test]
    fn test_run_stop_hook_quality_check_with_uncommitted_changes() {
        // Test quality check running when there are uncommitted changes (covers line 283)
        let mut runner = MockCommandRunner::new();
        let has_changes = CommandOutput {
            exit_code: 0,
            stdout: " file.rs | 10 ++++++++++\n".to_string(),
            stderr: String::new(),
        };
        let empty_success =
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() };
        let zero_commits =
            CommandOutput { exit_code: 0, stdout: "0\n".to_string(), stderr: String::new() };
        let quality_pass = CommandOutput {
            exit_code: 0,
            stdout: "All checks passed\n".to_string(),
            stderr: String::new(),
        };

        // First check_uncommitted_changes (fast path check)
        runner.expect("git", &["diff", "--stat"], has_changes.clone());
        runner.expect("git", &["diff", "--cached", "--stat"], empty_success.clone());
        runner.expect(
            "git",
            &["ls-files", "--others", "--exclude-standard"],
            empty_success.clone(),
        );
        runner.expect("git", &["rev-list", "--count", "@{upstream}..HEAD"], zero_commits.clone());

        // Second check_uncommitted_changes (main check)
        runner.expect("git", &["diff", "--stat"], has_changes);
        runner.expect("git", &["diff", "--cached", "--stat"], empty_success.clone());
        runner.expect(
            "git",
            &["ls-files", "--others", "--exclude-standard"],
            empty_success.clone(),
        );
        runner.expect("git", &["rev-list", "--count", "@{upstream}..HEAD"], zero_commits);

        // combined_diff for analysis
        runner.expect("git", &["diff", "--cached", "-U0"], empty_success.clone());
        runner.expect("git", &["diff", "-U0"], empty_success);

        // Quality check passes
        runner.expect("sh", &["-c", "just check"], quality_pass);

        let sub_agent = MockSubAgent::new();
        let input = crate::hooks::HookInput::default();
        let config = StopHookConfig {
            quality_check_enabled: true,
            quality_check_command: Some("just check".to_string()),
            ..Default::default()
        };

        let result = run_stop_hook(&input, &config, &runner, &sub_agent).unwrap();
        assert!(!result.allow_stop);
        // Should have run quality checks
        assert!(result.messages.iter().any(|m| m.contains("Running Quality Checks")));
    }
}
