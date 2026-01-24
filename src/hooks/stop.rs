//! Stop hook for code quality checks.
//!
//! This hook runs when Claude attempts to stop/exit. It implements:
//! - Uncommitted changes detection and blocking
//! - Quality checks via configured command
//! - Interactive question handling with sub-agent
//! - Task completion tracking

use crate::error::Result;
use crate::git::{self, GitStatus};
use crate::hooks::HookInput;
use crate::question::{is_continue_question, looks_like_question, truncate_for_context};
use crate::session;
use crate::tasks;
use crate::templates;
use crate::traits::{CommandRunner, QuestionContext, SubAgent, SubAgentDecision};
use crate::transcript::{self, is_simple_question, TranscriptInfo};
use chrono::{DateTime, Utc};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use tera::Context;

/// Magic string that allows stopping when work is complete but human input is required.
pub const HUMAN_INPUT_REQUIRED: &str =
    "I have completed all work that I can and require human input to proceed.";

/// Magic string that allows stopping when encountering an unsolvable problem.
pub const PROBLEM_NEEDS_USER: &str = "I have run into a problem I can't solve without user input.";

/// Time window for considering user as "recently active" (minutes).
pub const USER_RECENCY_MINUTES: u32 = 5;

/// Format a timestamp as a human-readable "time ago" string.
fn format_time_ago(time: DateTime<Utc>) -> String {
    let now = Utc::now();
    let duration = now.signed_duration_since(time);

    let minutes = duration.num_minutes();
    if minutes < 1 {
        "just now".to_string()
    } else if minutes == 1 {
        "1 minute ago".to_string()
    } else if minutes < 60 {
        format!("{minutes} minutes ago")
    } else {
        let hours = duration.num_hours();
        if hours == 1 {
            "1 hour ago".to_string()
        } else {
            format!("{hours} hours ago")
        }
    }
}

/// Configuration for the stop hook.
#[derive(Debug, Clone, Default)]
#[allow(clippy::struct_excessive_bools)] // Config structs legitimately have many boolean flags
pub struct StopHookConfig {
    /// Whether we're in a git repository.
    pub git_repo: bool,
    /// Skip quality checks (no-op by default until user configures).
    pub quality_check_enabled: bool,
    /// Command to run for quality checks.
    pub quality_check_command: Option<String>,
    /// Whether to require pushing before exit.
    pub require_push: bool,
    /// Base directory for file operations (defaults to current directory).
    /// Used by tests to avoid changing global CWD.
    pub base_dir: Option<PathBuf>,
    /// Whether to explain why stops are permitted.
    /// When true, always includes a message to the user explaining the reason.
    pub explain_stops: bool,
    /// Whether to automatically work on open tasks when user is idle.
    pub auto_work_on_tasks: bool,
    /// Minutes of user inactivity before prompting to work on tasks.
    pub auto_work_idle_minutes: u32,
}

impl StopHookConfig {
    /// Get the base directory for file operations, defaulting to current directory.
    fn base_dir(&self) -> &Path {
        self.base_dir.as_deref().unwrap_or_else(|| Path::new("."))
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

    /// Add an explanation for why the stop was permitted (user-facing message).
    /// Only adds the message if `explain` is true.
    #[must_use]
    pub fn with_explanation(self, explain: bool, reason: impl Into<String>) -> Self {
        if explain {
            self.with_message(format!("[Stop permitted: {}]", reason.into()))
        } else {
            self
        }
    }
}

/// Threshold for consecutive API errors before allowing stop.
/// Set to 1 to allow immediate stop on any API error (helps with debugging).
const API_ERROR_THRESHOLD: u32 = 1;

/// Maximum number of files to show before truncating with "... and X more"
const MAX_FILES_TO_SHOW: usize = 10;

/// Helper to add a file list to messages with truncation.
fn show_file_list(result: &mut StopHookResult, files: &[String], max_files: usize) {
    for (i, f) in files.iter().enumerate() {
        if i >= max_files {
            result.messages.push(format!("  ... and {} more", files.len() - max_files));
            break;
        }
        result.messages.push(format!("  {f}"));
    }
}

// =============================================================================
// Tier 1: Fast Exit Checks
// =============================================================================

/// Check if problem mode was active and should allow immediate exit.
///
/// When problem mode is active (from a previous "I have run into a problem" phrase),
/// we exit the mode and allow the stop unconditionally.
fn check_problem_mode_exit(config: &StopHookConfig) -> Result<Option<StopHookResult>> {
    if session::is_problem_mode_active(config.base_dir()) {
        session::exit_problem_mode(config.base_dir())?;
        let message = templates::render("messages/stop/problem_mode_exit.tera", &Context::new())
            .expect("problem_mode_exit.tera template should always render");
        return Ok(Some(
            StopHookResult::allow()
                .with_message(message)
                .with_explanation(config.explain_stops, "problem mode was active"),
        ));
    }
    Ok(None)
}

/// Check for API error loop and allow exit to prevent infinite loops.
///
/// If we've seen multiple consecutive API errors, allow the stop to prevent
/// the agent from getting stuck in an error loop.
fn check_api_error_loop(
    transcript_info: &TranscriptInfo,
    config: &StopHookConfig,
) -> Option<StopHookResult> {
    if transcript_info.consecutive_api_errors >= API_ERROR_THRESHOLD {
        let mut ctx = Context::new();
        ctx.insert("error_count", &transcript_info.consecutive_api_errors);
        let message = templates::render("messages/stop/api_error_loop.tera", &ctx)
            .expect("api_error_loop.tera template should always render");
        return Some(StopHookResult::allow().with_message(message).with_explanation(
            config.explain_stops,
            format!("{} consecutive API errors detected", transcript_info.consecutive_api_errors),
        ));
    }
    None
}

/// Check for simple Q&A exchange that should allow immediate exit.
///
/// If the last user message is a simple question and no modifications were made
/// since then, this is a clarifying Q&A - allow immediate stop (skip reflection).
/// This allows the agent to ask clarifying questions even if tasks are requested.
fn check_simple_qa_fast_path(
    transcript_info: &TranscriptInfo,
    config: &StopHookConfig,
) -> Option<StopHookResult> {
    if !transcript_info.has_modifying_tool_use_since_user {
        if let Some(ref last_user_msg) = transcript_info.last_user_message {
            if is_simple_question(last_user_msg) {
                if let Some(ref last_output) = transcript_info.last_assistant_output {
                    // Check if the output looks like a simple answer (not asking a question,
                    // and short enough that it's not a work summary)
                    let is_simple_answer =
                        !last_output.trim().ends_with('?') && last_output.lines().count() < 10;
                    if is_simple_answer {
                        return Some(StopHookResult::allow().with_explanation(
                            config.explain_stops,
                            "simple Q&A with no modifications since question",
                        ));
                    }
                }
            }
        }
    }
    None
}

/// Check for commit/push confirmation questions and auto-confirm them.
///
/// When the agent asks "Would you like me to commit/push?" just say yes.
/// This check is fast (string matching) so do it before git status checks.
fn check_commit_push_auto_confirm(
    transcript_info: &TranscriptInfo,
    config: &StopHookConfig,
) -> Option<StopHookResult> {
    if !config.git_repo {
        return None;
    }
    if let Some(ref output) = transcript_info.last_assistant_output {
        if let Some(response) = check_commit_push_question(output) {
            return Some(StopHookResult::block().with_inject(response));
        }
    }
    None
}

// =============================================================================
// Tier 2: Validation Checks
// =============================================================================

/// Check if validation is needed and run quality checks.
///
/// If modifying tools were used since last user message or validation,
/// run the validation command and block if it fails.
fn check_validation_required(
    config: &StopHookConfig,
    runner: &dyn CommandRunner,
) -> Result<Option<StopHookResult>> {
    if !session::needs_validation(config.base_dir()) {
        return Ok(None);
    }

    let Some(ref check_cmd) = config.quality_check_command else {
        return Ok(None);
    };

    // Run the validation command
    let output = runner.run("sh", &["-c", check_cmd], None)?;

    if output.exit_code != 0 {
        // Validation failed - block exit
        let mut result = StopHookResult::block()
            .with_message("# Validation Failed")
            .with_message("")
            .with_message(format!("The quality check command `{check_cmd}` found issues."))
            .with_message("")
            .with_message("Please fix these issues before continuing. Whether you introduced them or they were pre-existing, a clean quality check is part of completing your work well.");

        if !output.stdout.is_empty() {
            result = result.with_message("").with_message("**stdout:**");
            for line in output.stdout.lines().take(50) {
                result = result.with_message(format!("  {line}"));
            }
        }
        if !output.stderr.is_empty() {
            result = result.with_message("").with_message("**stderr:**");
            for line in output.stderr.lines().take(50) {
                result = result.with_message(format!("  {line}"));
            }
        }

        return Ok(Some(result));
    }

    // Validation passed - clear the marker
    session::clear_needs_validation(config.base_dir())?;
    Ok(None)
}

// =============================================================================
// Tier 3: Bypass Phrase Handling
// =============================================================================

/// Check for "I have run into a problem" phrase and enter problem mode.
///
/// When the agent says they've hit a problem they can't solve, enter problem mode
/// which blocks all tool use until the next stop.
fn check_problem_phrase(
    transcript_info: &TranscriptInfo,
    config: &StopHookConfig,
) -> Result<Option<StopHookResult>> {
    let Some(ref output) = transcript_info.last_assistant_output else {
        return Ok(None);
    };

    if output.contains(PROBLEM_NEEDS_USER) {
        // Enter problem mode - this blocks all tool use until next stop
        session::enter_problem_mode(config.base_dir())?;
        let message =
            templates::render("messages/stop/problem_mode_activated.tera", &Context::new())
                .expect("problem_mode_activated.tera template should always render");
        return Ok(Some(StopHookResult::block().with_message(message)));
    }
    Ok(None)
}

/// Check for incomplete requested tasks that block stopping.
///
/// Only checks if modifying work was done. Simple Q&A (no modifying tools) should
/// still be allowed.
fn check_requested_tasks_block(
    transcript_info: &TranscriptInfo,
    config: &StopHookConfig,
) -> Option<StopHookResult> {
    if !transcript_info.has_modifying_tool_use {
        return None;
    }
    check_incomplete_requested_tasks(config)
}

/// Check for "work complete" phrase and handle completion logic.
///
/// This handles the complex completion flow including:
/// - Checking for remaining ready tasks
/// - Checking for tasks blocked by unanswered questions
/// - Allowing bypass when all work is truly complete
fn check_completion_phrase(
    transcript_info: &TranscriptInfo,
    config: &StopHookConfig,
) -> Result<Option<StopHookResult>> {
    let Some(ref output) = transcript_info.last_assistant_output else {
        return Ok(None);
    };

    if !output.contains(HUMAN_INPUT_REQUIRED) {
        return Ok(None);
    }

    // Check if there are remaining tasks that can be worked on
    let tasks_count = tasks::count_ready_tasks(config.base_dir());

    if tasks_count > 0 {
        let mut result = StopHookResult::block()
            .with_message("# Exit Phrase Rejected")
            .with_message("")
            .with_message(format!("There are {tasks_count} task(s) ready to work on."))
            .with_message("")
            .with_message("Please work on the remaining tasks before exiting.");

        // Add task suggestion if available
        if let Some((id, title)) = tasks::suggest_task(config.base_dir()) {
            result = result
                .with_message("")
                .with_message(format!("Suggestion: Work on task \"{id}: {title}\" next."));
        }

        result = result
            .with_message("")
            .with_message("If you've hit a blocker you can't resolve, use this phrase instead:")
            .with_message("")
            .with_message(format!("  \"{PROBLEM_NEEDS_USER}\""));

        return Ok(Some(result));
    }

    // Check for tasks blocked by unanswered questions
    let question_blocked = tasks::get_question_blocked_tasks(config.base_dir());
    if !question_blocked.is_empty() {
        // There are tasks blocked by questions
        let has_shown_questions = session::has_questions_shown_marker(config.base_dir());

        if !has_shown_questions {
            // First time: show questions and ask agent to reflect
            session::set_questions_shown_marker(config.base_dir())?;

            let mut result = StopHookResult::block()
                .with_message("# Questions Require Reflection")
                .with_message("")
                .with_message("The following tasks are blocked by unanswered questions:")
                .with_message("");

            for (task_id, task_title, questions) in &question_blocked {
                result = result.with_message(format!("## Task: {task_id} - {task_title}"));
                for q in questions {
                    result = result.with_message(format!("  - [{}] {}", q.id, q.text));
                }
                result = result.with_message("");
            }

            result = result
                .with_message("Before asking the user, please reflect on these questions:")
                .with_message(
                    "- Can you now answer any of these questions yourself based on your work so far?",
                )
                .with_message("- Have you gained context that makes the answer clear?")
                .with_message("")
                .with_message(
                    "If you can answer a question, use the `answer_question` tool to record your answer.",
                )
                .with_message("If you truly cannot answer, you may try to exit again.");

            return Ok(Some(result));
        }

        // Second time: allow stop and present questions to user
        session::clear_questions_shown_marker(config.base_dir())?;

        // Collect unique questions across all blocked tasks
        let mut seen_questions = HashSet::new();
        let mut unique_questions = Vec::new();
        for (_, _, questions) in &question_blocked {
            for q in questions {
                if seen_questions.insert(q.id.clone()) {
                    unique_questions.push(q);
                }
            }
        }

        let mut result = StopHookResult::allow()
            .with_explanation(
                config.explain_stops,
                "all unblocked work complete, questions for user",
            )
            .with_message("# Questions for User")
            .with_message("")
            .with_message("The following questions need user input to unblock remaining tasks:")
            .with_message("");

        for q in unique_questions {
            result = result.with_message(format!("- [{}] {}", q.id, q.text));
        }

        result = result
            .with_message("")
            .with_message("Please answer these questions to unblock the remaining work.");

        return Ok(Some(result));
    }

    // Bypass allowed - work is complete
    Ok(Some(
        StopHookResult::allow()
            .with_explanation(config.explain_stops, "human input required phrase used"),
    ))
}

// =============================================================================
// Tier 4: Git State Checks
// =============================================================================

/// Check for uncommitted changes and block if present.
fn check_uncommitted_changes_block(
    config: &StopHookConfig,
    runner: &dyn CommandRunner,
    transcript_info: &TranscriptInfo,
    sub_agent: &dyn SubAgent,
) -> Result<Option<StopHookResult>> {
    if !config.git_repo {
        return Ok(None);
    }

    let git_status = git::check_uncommitted_changes(runner)?;

    if git_status.uncommitted.has_changes() {
        return Ok(Some(handle_uncommitted_changes(
            &git_status,
            config,
            runner,
            transcript_info,
            sub_agent,
        )?));
    }

    // Check if need to push
    if config.require_push && git_status.ahead_of_remote {
        let mut ctx = Context::new();
        ctx.insert("commits_ahead", &git_status.commits_ahead);
        let message = templates::render("messages/stop/unpushed_commits.tera", &ctx)
            .expect("unpushed_commits.tera template should always render");
        return Ok(Some(StopHookResult::block().with_message(message)));
    }

    Ok(None)
}

// =============================================================================
// Tier 5: Interactive Handling (check_interactive_question already exists)
// =============================================================================

/// Wrapper for `check_interactive_question` that returns Option for consistency.
fn check_interactive_question_block(
    transcript_info: &TranscriptInfo,
    sub_agent: &dyn SubAgent,
    config: &StopHookConfig,
) -> Result<Option<StopHookResult>> {
    check_interactive_question(transcript_info, sub_agent, config)
}

// =============================================================================
// Tier 6: Quality & Reflection Checks
// =============================================================================

/// Run quality checks if enabled and block on failure.
fn check_quality_gate(
    config: &StopHookConfig,
    runner: &dyn CommandRunner,
) -> Result<Option<StopHookResult>> {
    if !config.quality_check_enabled {
        return Ok(None);
    }

    let Some(ref cmd) = config.quality_check_command else {
        return Ok(None);
    };

    let output = runner.run("sh", &["-c", cmd], None)?;
    if !output.success() {
        return Ok(Some(
            StopHookResult::block()
                .with_message("# Quality Check Issues")
                .with_message("")
                .with_message("Quality checks found issues that need your attention. Fixing these (whether you introduced them or not) helps maintain a healthy codebase.")
                .with_message("")
                .with_message(truncate_output(&output.combined_output(), 50)),
        ));
    }
    Ok(None)
}

/// Check if reflection was already prompted and allow stop.
///
/// If the agent got the reflection prompt and is stopping again, allow it.
/// Also checks for incomplete requested tasks before allowing.
fn check_reflection_marker_allow(
    config: &StopHookConfig,
    transcript_info: &TranscriptInfo,
) -> Result<Option<StopHookResult>> {
    let base_dir = config.base_dir();
    if !session::has_reflect_marker(base_dir) {
        return Ok(None);
    }

    // Agent already got the reflection prompt and is stopping again - allow it
    session::clear_reflect_marker(base_dir)?;

    // Check for incomplete requested tasks before allowing stop (work was done)
    if let Some(result) = check_incomplete_requested_tasks(config) {
        return Ok(Some(result));
    }

    // Check if we should prompt to work on open tasks before allowing stop
    if let Some(result) = check_auto_work_tasks(config, transcript_info) {
        return Ok(Some(result));
    }

    Ok(Some(
        StopHookResult::allow()
            .with_explanation(config.explain_stops, "reflection already prompted on first stop"),
    ))
}

/// Skip reflection if agent is asking a question (waiting for user input).
///
/// This check is separate from `check_interactive_question` because that function
/// also checks user recency, but we want to skip reflection regardless of recency.
fn check_question_skip_reflection(
    transcript_info: &TranscriptInfo,
    config: &StopHookConfig,
) -> Option<StopHookResult> {
    if let Some(ref output) = transcript_info.last_assistant_output {
        if looks_like_question(output) {
            return Some(
                StopHookResult::allow()
                    .with_explanation(config.explain_stops, "agent is asking a question"),
            );
        }
    }
    None
}

/// Prompt for reflection on first stop if modifying tools were used.
fn check_reflection_prompt(
    transcript_info: &TranscriptInfo,
    config: &StopHookConfig,
) -> Result<Option<StopHookResult>> {
    if !transcript_info.has_modifying_tool_use {
        return Ok(None);
    }

    // Modifying tools were used, prompt for reflection
    session::set_reflect_marker(config.base_dir())?;
    Ok(Some(
        StopHookResult::block()
            .with_message("# Task Completion Check")
            .with_message("")
            .with_message(
                "Before exiting, carefully analyze whether you have fully completed the task.",
            )
            .with_message("")
            .with_message("If you have NOT completed the task:")
            .with_message("  - Continue working to finish it")
            .with_message("")
            .with_message("If you HAVE completed the task:")
            .with_message("  - Provide a clear, concise summary of what was done for the user")
            .with_message("  - Then stop again to exit"),
    ))
}

/// Check if we should prompt to work on open tasks (final check before allow).
fn check_auto_work_tasks_block(
    config: &StopHookConfig,
    transcript_info: &TranscriptInfo,
) -> Option<StopHookResult> {
    check_auto_work_tasks(config, transcript_info)
}

/// Run the stop hook.
///
/// This function orchestrates all stop checks in a specific order. Each check
/// can either allow the stop, block it, or pass through to the next check.
///
/// # Check Order
///
/// 1. **Tier 1 - Fast exits**: Problem mode, API errors, simple Q&A, auto-confirm commit/push
/// 2. **Tier 2 - Validation**: Run quality checks if configured
/// 3. **Tier 3 - Bypass phrases**: Problem phrase, requested tasks, completion phrase
/// 4. **Tier 4 - Git state**: Uncommitted changes block, unpushed commits check
/// 5. **Tier 5 - Interactive**: Question handling with sub-agent
/// 6. **Tier 6 - Quality & reflection**: Quality gate, reflection prompts, auto-work tasks
///
/// Note: A clean git repo is never a reason to allow stopping - it just means
/// git-related blocking conditions don't apply. All other checks still run.
///
/// # Errors
///
/// Returns an error if git commands, sub-agent calls, or file operations fail.
///
/// # Panics
///
/// Panics if embedded templates fail to render. Templates are embedded via
/// `include_str!` and verified by `test_all_embedded_templates_render`, so
/// this should only occur if a template has a bug that escaped tests.
pub fn run_stop_hook(
    input: &HookInput,
    config: &StopHookConfig,
    runner: &dyn CommandRunner,
    sub_agent: &dyn SubAgent,
) -> Result<StopHookResult> {
    // Parse transcript if available
    let transcript_info = input
        .transcript_path
        .as_ref()
        .and_then(|p| transcript::parse_transcript(Path::new(p)).ok())
        .unwrap_or_default();

    // =========================================================================
    // Tier 1: Fast Exit Checks
    // =========================================================================
    if let Some(r) = check_problem_mode_exit(config)? {
        return Ok(r);
    }
    if let Some(r) = check_api_error_loop(&transcript_info, config) {
        return Ok(r);
    }
    if let Some(r) = check_simple_qa_fast_path(&transcript_info, config) {
        return Ok(r);
    }
    if let Some(r) = check_commit_push_auto_confirm(&transcript_info, config) {
        return Ok(r);
    }

    // =========================================================================
    // Tier 2: Validation Checks
    // =========================================================================
    if let Some(r) = check_validation_required(config, runner)? {
        return Ok(r);
    }

    // =========================================================================
    // Tier 3: Bypass Phrase Handling
    // =========================================================================
    if let Some(r) = check_problem_phrase(&transcript_info, config)? {
        return Ok(r);
    }
    if let Some(r) = check_requested_tasks_block(&transcript_info, config) {
        return Ok(r);
    }
    if let Some(r) = check_completion_phrase(&transcript_info, config)? {
        return Ok(r);
    }

    // =========================================================================
    // Tier 4: Git State Checks
    // =========================================================================
    if let Some(r) = check_uncommitted_changes_block(config, runner, &transcript_info, sub_agent)? {
        return Ok(r);
    }

    // =========================================================================
    // Tier 5: Interactive Handling
    // =========================================================================
    if let Some(r) = check_interactive_question_block(&transcript_info, sub_agent, config)? {
        return Ok(r);
    }

    // =========================================================================
    // Tier 6: Quality & Reflection Checks
    // =========================================================================
    if let Some(r) = check_quality_gate(config, runner)? {
        return Ok(r);
    }
    if let Some(r) = check_reflection_marker_allow(config, &transcript_info)? {
        return Ok(r);
    }
    if let Some(r) = check_question_skip_reflection(&transcript_info, config) {
        return Ok(r);
    }
    if let Some(r) = check_reflection_prompt(&transcript_info, config)? {
        return Ok(r);
    }
    if let Some(r) = check_auto_work_tasks_block(config, &transcript_info) {
        return Ok(r);
    }

    // All checks passed - allow stop
    Ok(StopHookResult::allow().with_explanation(config.explain_stops, "no modifying tools used"))
}

/// Check if we should prompt the agent to work on open tasks.
///
/// This is triggered when:
/// 1. `auto_work_on_tasks` is enabled
/// 2. There are open, ready tasks in the database
/// 3. User has been idle for at least `auto_work_idle_minutes`
///
/// Returns Some(block result) if agent should work on tasks, None otherwise.
fn check_auto_work_tasks(
    config: &StopHookConfig,
    transcript_info: &TranscriptInfo,
) -> Option<StopHookResult> {
    // Skip if auto-work is disabled
    if !config.auto_work_on_tasks {
        return None;
    }

    // Check user idle time
    let user_idle_minutes = transcript_info.last_user_message_time.map_or(u32::MAX, |ts| {
        let now = Utc::now();
        let minutes = now.signed_duration_since(ts).num_minutes();
        // Clamp to u32 range (negative means user is in the future, treat as just now)
        u32::try_from(minutes.max(0)).unwrap_or(u32::MAX)
    }); // If no timestamp, treat as very idle

    if user_idle_minutes < config.auto_work_idle_minutes {
        return None;
    }

    // Check for open tasks
    let base_dir = config.base_dir();
    let ready_task_count = tasks::count_ready_tasks(base_dir);

    if ready_task_count == 0 {
        return None;
    }

    // User is idle and there are tasks - prompt to work on them
    let mut ctx = Context::new();
    ctx.insert("task_count", &ready_task_count);
    ctx.insert("idle_minutes", &user_idle_minutes);

    let message = templates::render("messages/stop/auto_work_tasks.tera", &ctx)
        .expect("auto_work_tasks.tera template should always render");

    Some(StopHookResult::block().with_message(message))
}

/// Check if there are incomplete requested tasks that block stopping.
///
/// Returns Some(block result) if there are incomplete requested tasks, None otherwise.
/// A task is considered "incomplete" if it's requested and not complete/abandoned,
/// unless it's blocked only on an unanswered question.
fn check_incomplete_requested_tasks(config: &StopHookConfig) -> Option<StopHookResult> {
    let incomplete = tasks::get_incomplete_requested_tasks(config.base_dir());

    if incomplete.is_empty() {
        // No incomplete requested tasks - also clear request mode since all are done
        tasks::clear_request_mode(config.base_dir());
        return None;
    }

    let mut result = StopHookResult::block()
        .with_message("# Requested Tasks Incomplete")
        .with_message("")
        .with_message(
            "The following tasks were requested by the user and must be completed before stopping:",
        )
        .with_message("");

    for (id, title, status) in &incomplete {
        result = result.with_message(format!("- [{status}] {id}: {title}"));
    }

    result = result
        .with_message("")
        .with_message("Please complete these tasks or, if blocked, link them to questions explaining the blocker.")
        .with_message("")
        .with_message("If you've hit a problem you cannot solve without user input:")
        .with_message("")
        .with_message(format!("  \"{PROBLEM_NEEDS_USER}\""));

    Some(result)
}

/// Check if the assistant's last message is asking about committing or pushing.
/// Returns Some(response) if we should auto-confirm, None otherwise.
fn check_commit_push_question(output: &str) -> Option<String> {
    // Get the last sentence/question from the output
    let trimmed = output.trim();

    // Check for commit confirmation questions
    if trimmed.ends_with("Would you like me to commit these changes?")
        || trimmed.ends_with("Would you like me to commit this?")
        || trimmed.ends_with("Would you like me to commit?")
        || trimmed.ends_with("Shall I commit these changes?")
        || trimmed.ends_with("Should I commit these changes?")
        || trimmed.ends_with("Ready to commit?")
    {
        return Some("Yes, please commit these changes.".to_string());
    }

    // Check for push confirmation questions
    if trimmed.ends_with("Would you like me to push these changes?")
        || trimmed.ends_with("Would you like me to push this?")
        || trimmed.ends_with("Would you like me to push?")
        || trimmed.ends_with("Shall I push these changes?")
        || trimmed.ends_with("Should I push these changes?")
        || trimmed.ends_with("Should I push?")
        || trimmed.ends_with("Ready to push?")
    {
        return Some("Yes, please push.".to_string());
    }

    // Check for combined commit and push
    if trimmed.ends_with("Would you like me to commit and push?")
        || trimmed.ends_with("Would you like me to commit and push these changes?")
        || trimmed.ends_with("Shall I commit and push?")
        || trimmed.ends_with("Should I commit and push?")
    {
        return Some("Yes, please commit and push.".to_string());
    }

    None
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

    result.messages.push("# Uncommitted Changes".to_string());
    result.messages.push(String::new());
    result.messages.push(format!(
        "You have {} that should be committed.",
        git_status.uncommitted.description()
    ));
    result.messages.push(String::new());

    // Show quality check results
    if !quality_passed {
        result.messages.push("## Quality Issues".to_string());
        result.messages.push(String::new());
        result
            .messages
            .push("Quality checks found issues. Please fix them - leaving the codebase in good shape is part of doing great work.".to_string());
        result.messages.push(String::new());
        if !quality_output.is_empty() {
            result.messages.push("### Output:".to_string());
            result.messages.push(String::new());
            result.messages.push(truncate_output(&quality_output, 50));
        }
    }

    // Show unstaged changes
    if !git_status.unstaged_files.is_empty() {
        result.messages.push("## Unstaged Changes".to_string());
        result.messages.push(String::new());
        result.messages.push("The following files have been modified:".to_string());
        result.messages.push(String::new());
        show_file_list(&mut result, &git_status.unstaged_files, MAX_FILES_TO_SHOW);
        result.messages.push(String::new());
    }

    // Show staged changes
    if !git_status.staged_files.is_empty() {
        result.messages.push("## Staged Changes".to_string());
        result.messages.push(String::new());
        result.messages.push("The following files are staged for commit:".to_string());
        result.messages.push(String::new());
        show_file_list(&mut result, &git_status.staged_files, MAX_FILES_TO_SHOW);
        result.messages.push(String::new());
    }

    // Show untracked files
    if !git_status.untracked_files.is_empty() {
        result.messages.push("## Untracked Files".to_string());
        result.messages.push(String::new());
        result.messages.push("The following files are not tracked by git:".to_string());
        result.messages.push(String::new());
        show_file_list(&mut result, &git_status.untracked_files, MAX_FILES_TO_SHOW);
        result.messages.push(String::new());
        result.messages.push("Either `git add` them or add them to .gitignore".to_string());
        result.messages.push(String::new());
    }

    // Instructions - dynamically number steps based on what's enabled
    result.messages.push("Before stopping, please:".to_string());
    result.messages.push(String::new());
    let mut step = 1;
    result
        .messages
        .push(format!("{step}. Run `git status` to check for files that should be gitignored"));
    step += 1;
    if config.quality_check_enabled {
        result.messages.push(format!("{step}. Run quality checks to verify they pass"));
        step += 1;
    }
    result.messages.push(format!("{step}. Stage your changes: `git add <files>`"));
    step += 1;
    result
        .messages
        .push(format!("{step}. Commit with a descriptive message: `git commit -m '...'`"));
    step += 1;
    if config.require_push {
        result.messages.push(format!("{step}. Push to remote: `git push`"));
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

/// Check for interactive question handling.
fn check_interactive_question(
    transcript_info: &TranscriptInfo,
    sub_agent: &dyn SubAgent,
    config: &StopHookConfig,
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
    let truncated_output = truncate_for_context(output, 2000);

    // Fast path: Auto-answer "should I continue?" questions
    if is_continue_question(truncated_output) {
        return Ok(Some(
            StopHookResult::block()
                .with_message("# Fast path: Auto-answering continue question")
                .with_inject("Yes, please continue."),
        ));
    }

    // Build question context for sub-agent
    let question_context = QuestionContext {
        assistant_output: truncated_output.to_string(),
        user_recency_minutes: USER_RECENCY_MINUTES,
        user_last_active: transcript_info.last_user_message_time.map(format_time_ago),
        has_modifications_since_user: transcript_info.has_modifying_tool_use_since_user,
    };

    // Run sub-agent decision
    let decision = sub_agent.decide_on_question(&question_context)?;

    match decision {
        SubAgentDecision::AllowStop(reason) => {
            let mut result = StopHookResult::allow()
                .with_message("# Allowing Stop for User Interaction")
                .with_message("")
                .with_message("Agent appears to be asking a question.");
            if let Some(ref r) = reason {
                result.messages.push(format!("Reason: {r}"));
            }
            let explanation = reason.unwrap_or_else(|| "agent asking question".to_string());
            result = result.with_explanation(config.explain_stops, explanation);
            Ok(Some(result))
        }
        SubAgentDecision::Answer(answer) => Ok(Some(
            StopHookResult::block()
                .with_message("# Sub-agent Response")
                .with_message("")
                .with_message(&answer)
                .with_message("")
                .with_message("---")
                .with_message("Continuing work...")
                .with_inject(answer),
        )),
        SubAgentDecision::Continue => Ok(None),
    }
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
    use chrono::Duration;
    use tempfile::TempDir;

    #[test]
    fn test_format_time_ago_just_now() {
        let now = Utc::now();
        assert_eq!(format_time_ago(now), "just now");
        assert_eq!(format_time_ago(now - Duration::seconds(30)), "just now");
    }

    #[test]
    fn test_format_time_ago_one_minute() {
        let one_min_ago = Utc::now() - Duration::minutes(1);
        assert_eq!(format_time_ago(one_min_ago), "1 minute ago");
    }

    #[test]
    fn test_format_time_ago_minutes() {
        let five_min_ago = Utc::now() - Duration::minutes(5);
        assert_eq!(format_time_ago(five_min_ago), "5 minutes ago");

        let thirty_min_ago = Utc::now() - Duration::minutes(30);
        assert_eq!(format_time_ago(thirty_min_ago), "30 minutes ago");

        let fifty_nine_min_ago = Utc::now() - Duration::minutes(59);
        assert_eq!(format_time_ago(fifty_nine_min_ago), "59 minutes ago");
    }

    #[test]
    fn test_format_time_ago_one_hour() {
        let one_hour_ago = Utc::now() - Duration::hours(1);
        assert_eq!(format_time_ago(one_hour_ago), "1 hour ago");
    }

    #[test]
    fn test_format_time_ago_hours() {
        let two_hours_ago = Utc::now() - Duration::hours(2);
        assert_eq!(format_time_ago(two_hours_ago), "2 hours ago");

        let five_hours_ago = Utc::now() - Duration::hours(5);
        assert_eq!(format_time_ago(five_hours_ago), "5 hours ago");
    }

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
        let dir = tempfile::TempDir::new().unwrap();
        let runner = mock_clean_git();
        let sub_agent = MockSubAgent::new();
        let input = crate::hooks::HookInput::default();
        let config =
            StopHookConfig { base_dir: Some(dir.path().to_path_buf()), ..Default::default() };

        let result = run_stop_hook(&input, &config, &runner, &sub_agent).unwrap();
        assert!(result.allow_stop);
        assert_eq!(result.exit_code, 0);
    }

    #[test]
    fn test_run_stop_hook_blocks_with_incomplete_requested_tasks_and_modifying_tools() {
        // Critical test: with modifying tools used and incomplete requested tasks,
        // stop must be blocked regardless of git status.
        use crate::tasks::{Priority, SqliteTaskStore, TaskStore};

        let dir = tempfile::TempDir::new().unwrap();
        let base = dir.path();

        // Set up database directory and create a requested task
        let db_path = crate::paths::project_db_path(base);
        std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();
        let store = SqliteTaskStore::for_project(base).unwrap();
        let task = store.create_task("Critical task", "Must complete", Priority::High).unwrap();
        store.request_tasks(&[&task.id]).unwrap();

        // Mock a clean git repo (no changes, not ahead of remote)
        let runner = mock_clean_git();
        let sub_agent = MockSubAgent::new();

        // Create a transcript with modifying tool use - THIS IS KEY
        let transcript_content = r#"{"type": "user", "timestamp": "2024-01-01T12:00:00Z", "message": {"role": "user", "content": "Do the task"}}
{"type": "assistant", "message": {"content": [{"type": "tool_use", "name": "Edit", "id": "1"}]}}
{"type": "assistant", "message": {"content": [{"type": "text", "text": "I made some changes."}]}}
"#;
        let transcript_file = base.join("transcript.jsonl");
        std::fs::write(&transcript_file, transcript_content).unwrap();

        let input = crate::hooks::HookInput {
            transcript_path: Some(transcript_file.to_string_lossy().to_string()),
            ..Default::default()
        };
        let config = StopHookConfig {
            base_dir: Some(base.to_path_buf()),
            git_repo: true,
            explain_stops: true,
            ..Default::default()
        };

        let result = run_stop_hook(&input, &config, &runner, &sub_agent).unwrap();

        // MUST block because there's an incomplete requested task
        assert!(
            !result.allow_stop,
            "Expected stop to be BLOCKED with incomplete requested tasks, but it was allowed. \
             Messages: {:?}",
            result.messages
        );
        assert!(
            result.messages.iter().any(|m| m.contains("Requested Tasks Incomplete")),
            "Expected 'Requested Tasks Incomplete' message but got: {:?}",
            result.messages
        );
    }

    #[test]
    fn test_run_stop_hook_allows_without_modifying_tools() {
        // Without modifying tools, stop is allowed even with requested tasks
        // (agent hasn't started working yet)
        use crate::tasks::{Priority, SqliteTaskStore, TaskStore};

        let dir = tempfile::TempDir::new().unwrap();
        let base = dir.path();

        // Set up database directory and create a requested task
        let db_path = crate::paths::project_db_path(base);
        std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();
        let store = SqliteTaskStore::for_project(base).unwrap();
        let task = store.create_task("Task", "Description", Priority::High).unwrap();
        store.request_tasks(&[&task.id]).unwrap();

        // Mock a clean git repo
        let runner = mock_clean_git();
        let sub_agent = MockSubAgent::new();
        let input = crate::hooks::HookInput::default(); // No transcript = no modifying tools
        let config = StopHookConfig {
            base_dir: Some(base.to_path_buf()),
            git_repo: true,
            explain_stops: true,
            ..Default::default()
        };

        let result = run_stop_hook(&input, &config, &runner, &sub_agent).unwrap();

        // Should allow because no modifying tools were used
        assert!(
            result.allow_stop,
            "Expected stop to be allowed without modifying tools. Messages: {:?}",
            result.messages
        );
    }

    #[test]
    fn test_run_stop_hook_allows_with_completed_requested_tasks_and_modifying_tools() {
        // After completing a requested task, should allow exit even with modifying tools
        use crate::tasks::{Priority, SqliteTaskStore, Status, TaskStore, TaskUpdate};

        let dir = tempfile::TempDir::new().unwrap();
        let base = dir.path();

        // Set reflection marker since modifying tools trigger reflection on first stop
        session::set_reflect_marker(base).unwrap();

        // Set up database directory and create a requested task, then complete it
        let db_path = crate::paths::project_db_path(base);
        std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();
        let store = SqliteTaskStore::for_project(base).unwrap();
        let task = store.create_task("Task", "Description", Priority::High).unwrap();
        store.request_tasks(&[&task.id]).unwrap();
        store
            .update_task(
                &task.id,
                TaskUpdate { status: Some(Status::Complete), ..Default::default() },
            )
            .unwrap();

        // Mock a clean git repo
        let runner = mock_clean_git();
        let sub_agent = MockSubAgent::new();

        // Create a transcript with modifying tool use
        let transcript_content = r#"{"type": "user", "timestamp": "2024-01-01T12:00:00Z", "message": {"role": "user", "content": "Do the task"}}
{"type": "assistant", "message": {"content": [{"type": "tool_use", "name": "Edit", "id": "1"}]}}
{"type": "assistant", "message": {"content": [{"type": "text", "text": "Done!"}]}}
"#;
        let transcript_file = base.join("transcript.jsonl");
        std::fs::write(&transcript_file, transcript_content).unwrap();

        let input = crate::hooks::HookInput {
            transcript_path: Some(transcript_file.to_string_lossy().to_string()),
            ..Default::default()
        };
        let config = StopHookConfig {
            base_dir: Some(base.to_path_buf()),
            git_repo: true,
            explain_stops: true,
            ..Default::default()
        };

        let result = run_stop_hook(&input, &config, &runner, &sub_agent).unwrap();

        // Should allow because task is completed
        assert!(
            result.allow_stop,
            "Expected stop to be allowed with completed task. Messages: {:?}",
            result.messages
        );
    }

    #[test]
    fn test_run_stop_hook_allows_with_question_blocked_requested_tasks() {
        // If a requested task is blocked by an unanswered question, allow exit
        // (user needs to answer before work can continue)
        use crate::tasks::{Priority, SqliteTaskStore, TaskStore};

        let dir = tempfile::TempDir::new().unwrap();
        let base = dir.path();

        // Set reflection marker since modifying tools trigger reflection on first stop
        session::set_reflect_marker(base).unwrap();

        // Set up database directory and create a requested task blocked by question
        let db_path = crate::paths::project_db_path(base);
        std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();
        let store = SqliteTaskStore::for_project(base).unwrap();
        let task = store.create_task("Task", "Description", Priority::High).unwrap();
        let question = store.create_question("What should I do?").unwrap();
        store.link_task_to_question(&task.id, &question.id).unwrap();
        store.request_tasks(&[&task.id]).unwrap();

        // Mock a clean git repo
        let runner = mock_clean_git();
        let sub_agent = MockSubAgent::new();

        // Create a transcript with modifying tool use
        let transcript_content = r#"{"type": "user", "timestamp": "2024-01-01T12:00:00Z", "message": {"role": "user", "content": "Do the task"}}
{"type": "assistant", "message": {"content": [{"type": "tool_use", "name": "Edit", "id": "1"}]}}
{"type": "assistant", "message": {"content": [{"type": "text", "text": "I need to ask a question."}]}}
"#;
        let transcript_file = base.join("transcript.jsonl");
        std::fs::write(&transcript_file, transcript_content).unwrap();

        let input = crate::hooks::HookInput {
            transcript_path: Some(transcript_file.to_string_lossy().to_string()),
            ..Default::default()
        };
        let config = StopHookConfig {
            base_dir: Some(base.to_path_buf()),
            git_repo: true,
            explain_stops: true,
            ..Default::default()
        };

        let result = run_stop_hook(&input, &config, &runner, &sub_agent).unwrap();

        // Should allow because task is blocked on a question
        assert!(
            result.allow_stop,
            "Expected stop to be allowed with question-blocked task. Messages: {:?}",
            result.messages
        );
    }

    #[test]
    fn test_explain_stops_adds_explanation_message() {
        let dir = tempfile::TempDir::new().unwrap();
        let runner = mock_clean_git();
        let sub_agent = MockSubAgent::new();
        let input = crate::hooks::HookInput::default();
        let config = StopHookConfig {
            base_dir: Some(dir.path().to_path_buf()),
            explain_stops: true,
            ..Default::default()
        };

        let result = run_stop_hook(&input, &config, &runner, &sub_agent).unwrap();
        assert!(result.allow_stop);
        // Should include an explanation message
        let has_explanation = result.messages.iter().any(|m| m.starts_with("[Stop permitted:"));
        assert!(has_explanation, "Expected explanation message, got: {:?}", result.messages);
    }

    #[test]
    fn test_explain_stops_disabled_no_message() {
        let dir = tempfile::TempDir::new().unwrap();
        let runner = mock_clean_git();
        let sub_agent = MockSubAgent::new();
        let input = crate::hooks::HookInput::default();
        let config = StopHookConfig {
            base_dir: Some(dir.path().to_path_buf()),
            explain_stops: false,
            ..Default::default()
        };

        let result = run_stop_hook(&input, &config, &runner, &sub_agent).unwrap();
        assert!(result.allow_stop);
        // Should NOT include an explanation message
        let has_explanation = result.messages.iter().any(|m| m.starts_with("[Stop permitted:"));
        assert!(!has_explanation, "Unexpected explanation message: {:?}", result.messages);
    }

    fn mock_uncommitted_changes() -> MockCommandRunner {
        let mut runner = MockCommandRunner::new();
        let has_changes = CommandOutput {
            exit_code: 0,
            stdout: " file.rs | 10 ++++++++++\n".to_string(),
            stderr: String::new(),
        };
        let file_list =
            CommandOutput { exit_code: 0, stdout: "file.rs\n".to_string(), stderr: String::new() };
        let empty_success =
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() };
        let zero_commits =
            CommandOutput { exit_code: 0, stdout: "0\n".to_string(), stderr: String::new() };

        // First check_uncommitted_changes (fast path check)
        runner.expect("git", &["diff", "--stat"], has_changes.clone());
        runner.expect("git", &["diff", "--name-only"], file_list.clone()); // Only when has_unstaged
        runner.expect("git", &["diff", "--cached", "--stat"], empty_success.clone());
        // No --cached --name-only when no staged changes
        runner.expect(
            "git",
            &["ls-files", "--others", "--exclude-standard"],
            empty_success.clone(),
        );
        runner.expect("git", &["rev-list", "--count", "@{upstream}..HEAD"], zero_commits.clone());

        // Second check_uncommitted_changes (main check)
        runner.expect("git", &["diff", "--stat"], has_changes);
        runner.expect("git", &["diff", "--name-only"], file_list); // Only when has_unstaged
        runner.expect("git", &["diff", "--cached", "--stat"], empty_success.clone());
        // No --cached --name-only when no staged changes
        runner.expect("git", &["ls-files", "--others", "--exclude-standard"], empty_success);
        runner.expect("git", &["rev-list", "--count", "@{upstream}..HEAD"], zero_commits);

        runner
    }

    fn mock_staged_changes() -> MockCommandRunner {
        let mut runner = MockCommandRunner::new();
        let has_staged = CommandOutput {
            exit_code: 0,
            stdout: " src/main.rs | 3 +++\n".to_string(),
            stderr: String::new(),
        };
        let staged_file_list = CommandOutput {
            exit_code: 0,
            stdout: "src/main.rs\n".to_string(),
            stderr: String::new(),
        };
        let empty_success =
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() };
        let zero_commits =
            CommandOutput { exit_code: 0, stdout: "0\n".to_string(), stderr: String::new() };

        // First check_uncommitted_changes (fast path check)
        runner.expect("git", &["diff", "--stat"], empty_success.clone()); // No unstaged
        runner.expect("git", &["diff", "--cached", "--stat"], has_staged.clone());
        runner.expect("git", &["diff", "--cached", "--name-only"], staged_file_list.clone());
        runner.expect(
            "git",
            &["ls-files", "--others", "--exclude-standard"],
            empty_success.clone(),
        );
        runner.expect("git", &["rev-list", "--count", "@{upstream}..HEAD"], zero_commits.clone());

        // Second check_uncommitted_changes (main check)
        runner.expect("git", &["diff", "--stat"], empty_success.clone());
        runner.expect("git", &["diff", "--cached", "--stat"], has_staged);
        runner.expect("git", &["diff", "--cached", "--name-only"], staged_file_list);
        runner.expect("git", &["ls-files", "--others", "--exclude-standard"], empty_success);
        runner.expect("git", &["rev-list", "--count", "@{upstream}..HEAD"], zero_commits);

        runner
    }

    #[test]
    fn test_run_stop_hook_shows_staged_files() {
        let dir = tempfile::TempDir::new().unwrap();
        let runner = mock_staged_changes();
        let sub_agent = MockSubAgent::new();
        let input = crate::hooks::HookInput::default();
        let config = StopHookConfig {
            git_repo: true,
            base_dir: Some(dir.path().to_path_buf()),
            ..Default::default()
        };

        let result = run_stop_hook(&input, &config, &runner, &sub_agent).unwrap();
        assert!(!result.allow_stop);
        assert!(result.messages.iter().any(|m| m.contains("Staged Changes")));
        assert!(result.messages.iter().any(|m| m.contains("src/main.rs")));
    }

    #[test]
    fn test_run_stop_hook_uncommitted_changes_blocks() {
        let dir = tempfile::TempDir::new().unwrap();
        let runner = mock_uncommitted_changes();
        let sub_agent = MockSubAgent::new();
        let input = crate::hooks::HookInput::default();
        let config = StopHookConfig {
            git_repo: true,
            base_dir: Some(dir.path().to_path_buf()),
            ..Default::default()
        };

        let result = run_stop_hook(&input, &config, &runner, &sub_agent).unwrap();
        assert!(!result.allow_stop);
        assert_eq!(result.exit_code, 2);
        assert!(result.messages.iter().any(|m| m.contains("Uncommitted Changes")));
    }

    #[test]
    fn test_run_stop_hook_skips_git_when_not_git_repo() {
        // When git_repo is false, git checks should be skipped entirely.
        // This test verifies that NO git commands are called when git_repo: false.
        let dir = tempfile::TempDir::new().unwrap();
        let runner = MockCommandRunner::new(); // No git expectations set
        let sub_agent = MockSubAgent::new();
        let input = crate::hooks::HookInput::default();
        let config = StopHookConfig {
            git_repo: false, // Not a git repo - git checks should be skipped
            base_dir: Some(dir.path().to_path_buf()),
            ..Default::default()
        };

        // Should succeed without calling any git commands
        let result = run_stop_hook(&input, &config, &runner, &sub_agent).unwrap();
        assert!(result.allow_stop);
        runner.verify(); // Verify no unexpected commands were called
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
    fn test_stop_hook_result_with_explanation_enabled() {
        let result = StopHookResult::allow().with_explanation(true, "test reason");
        assert_eq!(result.messages.len(), 1);
        assert_eq!(result.messages[0], "[Stop permitted: test reason]");
    }

    #[test]
    fn test_stop_hook_result_with_explanation_disabled() {
        let result = StopHookResult::allow().with_explanation(false, "test reason");
        assert!(result.messages.is_empty());
    }

    #[test]
    fn test_check_auto_work_tasks_disabled() {
        let config = StopHookConfig { auto_work_on_tasks: false, ..Default::default() };
        let transcript = TranscriptInfo::default();

        let result = check_auto_work_tasks(&config, &transcript);
        assert!(result.is_none());
    }

    #[test]
    fn test_check_auto_work_tasks_user_recently_active() {
        use chrono::Utc;

        let config = StopHookConfig {
            auto_work_on_tasks: true,
            auto_work_idle_minutes: 15,
            ..Default::default()
        };
        let transcript = TranscriptInfo {
            last_user_message_time: Some(Utc::now()), // Just now
            ..Default::default()
        };

        let result = check_auto_work_tasks(&config, &transcript);
        assert!(result.is_none()); // User is active, shouldn't block
    }

    #[test]
    fn test_check_auto_work_tasks_no_tasks() {
        use chrono::Utc;
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();

        let config = StopHookConfig {
            auto_work_on_tasks: true,
            auto_work_idle_minutes: 15,
            base_dir: Some(dir.path().to_path_buf()),
            ..Default::default()
        };
        let transcript = TranscriptInfo {
            last_user_message_time: Some(Utc::now() - chrono::Duration::minutes(30)),
            ..Default::default()
        };

        let result = check_auto_work_tasks(&config, &transcript);
        assert!(result.is_none()); // No tasks, shouldn't block
    }

    #[test]
    fn test_check_auto_work_tasks_blocks_with_tasks() {
        use crate::tasks::models::Priority;
        use crate::tasks::store::{SqliteTaskStore, TaskStore};
        use chrono::Utc;
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let db_path = crate::paths::project_db_path(dir.path());

        // Create a ready task
        let store = SqliteTaskStore::new(&db_path).unwrap();
        store.create_task("Test task", "description", Priority::Medium).unwrap();

        let config = StopHookConfig {
            auto_work_on_tasks: true,
            auto_work_idle_minutes: 15,
            base_dir: Some(dir.path().to_path_buf()),
            ..Default::default()
        };
        let transcript = TranscriptInfo {
            last_user_message_time: Some(Utc::now() - chrono::Duration::minutes(30)),
            ..Default::default()
        };

        let result = check_auto_work_tasks(&config, &transcript);
        assert!(result.is_some()); // Has tasks and user is idle - should block
        let result = result.unwrap();
        assert!(!result.allow_stop);
        assert!(result.messages.iter().any(|m| m.contains("Open Tasks")));
    }

    #[test]
    fn test_check_auto_work_tasks_no_timestamp_treats_as_idle() {
        use crate::tasks::models::Priority;
        use crate::tasks::store::{SqliteTaskStore, TaskStore};
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let db_path = crate::paths::project_db_path(dir.path());

        // Create a ready task
        let store = SqliteTaskStore::new(&db_path).unwrap();
        store.create_task("Test task", "description", Priority::Medium).unwrap();

        let config = StopHookConfig {
            auto_work_on_tasks: true,
            auto_work_idle_minutes: 15,
            base_dir: Some(dir.path().to_path_buf()),
            ..Default::default()
        };
        let transcript = TranscriptInfo {
            last_user_message_time: None, // No timestamp
            ..Default::default()
        };

        let result = check_auto_work_tasks(&config, &transcript);
        assert!(result.is_some()); // No timestamp = very idle, should block
    }

    #[test]
    fn test_stop_hook_auto_work_blocks_after_reflection() {
        // Test that auto-work blocks after reflection marker is set (covers line 465)
        use crate::tasks::models::Priority;
        use crate::tasks::store::{SqliteTaskStore, TaskStore};
        use crate::testing::{MockCommandRunner, MockSubAgent};
        use chrono::Utc;
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let base = dir.path();
        let db_path = crate::paths::project_db_path(base);

        // Set up reflection marker (simulating previous stop)
        crate::session::set_reflect_marker(base).unwrap();

        // Create a ready task
        let store = SqliteTaskStore::new(&db_path).unwrap();
        store.create_task("Test task", "description", Priority::Medium).unwrap();

        let runner = MockCommandRunner::new();
        let sub_agent = MockSubAgent::new();

        let config = StopHookConfig {
            auto_work_on_tasks: true,
            auto_work_idle_minutes: 15,
            base_dir: Some(base.to_path_buf()),
            ..Default::default()
        };

        // Create transcript showing user has been idle
        let transcript_path = base.join("transcript.jsonl");
        let old_time = Utc::now() - chrono::Duration::minutes(30);
        std::fs::write(
            &transcript_path,
            format!(
                r#"{{"type":"user","content":[{{"type":"text","text":"test"}}],"timestamp":"{}"}}"#,
                old_time.to_rfc3339()
            ),
        )
        .unwrap();

        let input = crate::hooks::HookInput {
            transcript_path: Some(transcript_path.to_string_lossy().to_string()),
            ..Default::default()
        };

        let result = run_stop_hook(&input, &config, &runner, &sub_agent).unwrap();

        // Should block to work on tasks
        assert!(!result.allow_stop);
        assert!(result.messages.iter().any(|m| m.contains("Open Tasks")));
    }

    #[test]
    fn test_stop_hook_auto_work_blocks_without_modifying_tools() {
        // Test that auto-work blocks when no modifying tools used (covers line 502)
        use crate::tasks::models::Priority;
        use crate::tasks::store::{SqliteTaskStore, TaskStore};
        use crate::testing::{MockCommandRunner, MockSubAgent};
        use chrono::Utc;
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let base = dir.path();
        let db_path = crate::paths::project_db_path(base);

        // Create a ready task
        let store = SqliteTaskStore::new(&db_path).unwrap();
        store.create_task("Test task", "description", Priority::Medium).unwrap();

        let runner = MockCommandRunner::new();
        let sub_agent = MockSubAgent::new();

        let config = StopHookConfig {
            auto_work_on_tasks: true,
            auto_work_idle_minutes: 15,
            base_dir: Some(base.to_path_buf()),
            // Not a git repo, so no git checks
            git_repo: false,
            ..Default::default()
        };

        // Create transcript showing user has been idle
        let transcript_path = base.join("transcript.jsonl");
        let old_time = Utc::now() - chrono::Duration::minutes(30);
        std::fs::write(
            &transcript_path,
            format!(
                r#"{{"type":"user","content":[{{"type":"text","text":"test"}}],"timestamp":"{}"}}"#,
                old_time.to_rfc3339()
            ),
        )
        .unwrap();

        let input = crate::hooks::HookInput {
            transcript_path: Some(transcript_path.to_string_lossy().to_string()),
            ..Default::default()
        };

        let result = run_stop_hook(&input, &config, &runner, &sub_agent).unwrap();

        // Should block to work on tasks (no modifying tools = would normally allow, but tasks exist)
        assert!(!result.allow_stop);
        assert!(result.messages.iter().any(|m| m.contains("Open Tasks")));
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
        let dir = TempDir::new().unwrap();
        let runner = mock_clean_with_ahead();
        let sub_agent = MockSubAgent::new();
        let input = crate::hooks::HookInput::default();
        let config = StopHookConfig {
            git_repo: true,
            require_push: true,
            base_dir: Some(dir.path().to_path_buf()),
            ..Default::default()
        };

        let result = run_stop_hook(&input, &config, &runner, &sub_agent).unwrap();
        assert!(!result.allow_stop);
        assert_eq!(result.exit_code, 2);
        assert!(result.messages.iter().any(|m| m.contains("Unpushed Commits")));
    }

    #[test]
    fn test_run_stop_hook_unpushed_allowed_without_require_push() {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // Set up reflect marker so the reflection check is skipped
        // (this test is about require_push behavior, not reflection)
        session::set_reflect_marker(base).unwrap();

        let runner = mock_clean_with_ahead();
        let sub_agent = MockSubAgent::new();
        let input = crate::hooks::HookInput::default();
        let config = StopHookConfig {
            git_repo: true,
            require_push: false,
            base_dir: Some(base.to_path_buf()),
            ..Default::default()
        };

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
        let file_list =
            CommandOutput { exit_code: 0, stdout: "file.rs\n".to_string(), stderr: String::new() };
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
        runner.expect("git", &["diff", "--name-only"], file_list.clone());
        runner.expect("git", &["diff", "--cached", "--stat"], empty_success.clone());
        runner.expect(
            "git",
            &["ls-files", "--others", "--exclude-standard"],
            untracked_files.clone(),
        );
        runner.expect("git", &["rev-list", "--count", "@{upstream}..HEAD"], zero_commits.clone());

        // Second check_uncommitted_changes (main check)
        runner.expect("git", &["diff", "--stat"], has_changes);
        runner.expect("git", &["diff", "--name-only"], file_list);
        runner.expect("git", &["diff", "--cached", "--stat"], empty_success);
        runner.expect("git", &["ls-files", "--others", "--exclude-standard"], untracked_files);
        runner.expect("git", &["rev-list", "--count", "@{upstream}..HEAD"], zero_commits);

        runner
    }

    #[test]
    fn test_run_stop_hook_shows_untracked_files() {
        let dir = TempDir::new().unwrap();
        let runner = mock_uncommitted_with_untracked();
        let sub_agent = MockSubAgent::new();
        let input = crate::hooks::HookInput::default();
        let config = StopHookConfig {
            git_repo: true,
            base_dir: Some(dir.path().to_path_buf()),
            ..Default::default()
        };

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
        let file_list =
            CommandOutput { exit_code: 0, stdout: "file.rs\n".to_string(), stderr: String::new() };
        let empty_success =
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() };
        let zero_commits =
            CommandOutput { exit_code: 0, stdout: "0\n".to_string(), stderr: String::new() };

        // check_uncommitted_changes - detects changes
        runner.expect("git", &["diff", "--stat"], has_changes);
        runner.expect("git", &["diff", "--name-only"], file_list);
        runner.expect("git", &["diff", "--cached", "--stat"], empty_success.clone());
        runner.expect("git", &["ls-files", "--others", "--exclude-standard"], empty_success);
        runner.expect("git", &["rev-list", "--count", "@{upstream}..HEAD"], zero_commits);

        // Quality check command (runs inside handle_uncommitted_changes)
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
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let runner = mock_uncommitted_with_quality_checks();
        let sub_agent = MockSubAgent::new();
        let input = crate::hooks::HookInput::default();
        let config = StopHookConfig {
            git_repo: true,
            quality_check_enabled: true,
            quality_check_command: Some("just check".to_string()),
            base_dir: Some(dir.path().to_path_buf()),
            ..Default::default()
        };

        let result = run_stop_hook(&input, &config, &runner, &sub_agent).unwrap();
        assert!(!result.allow_stop);
        assert!(result.messages.iter().any(|m| m.contains("Quality Issues")));
    }

    #[test]
    fn test_check_interactive_question_no_output() {
        let transcript_info = TranscriptInfo::default();
        let sub_agent = MockSubAgent::new();

        let result =
            check_interactive_question(&transcript_info, &sub_agent, &StopHookConfig::default())
                .unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_check_interactive_question_not_a_question() {
        let transcript_info = TranscriptInfo {
            last_assistant_output: Some("This is just a statement.".to_string()),
            last_user_message_time: None,
            has_api_error: false,
            consecutive_api_errors: 0,
            has_modifying_tool_use: false,
            has_modifying_tool_use_since_user: false,
            first_user_message: None,
            last_user_message: None,
        };
        let sub_agent = MockSubAgent::new();

        let result =
            check_interactive_question(&transcript_info, &sub_agent, &StopHookConfig::default())
                .unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_check_interactive_question_user_not_active() {
        use chrono::{Duration, Utc};
        let transcript_info = TranscriptInfo {
            last_assistant_output: Some("Would you like me to continue?".to_string()),
            // User was active 10 minutes ago (beyond the 5-minute threshold)
            last_user_message_time: Some(Utc::now() - Duration::minutes(10)),
            has_api_error: false,
            consecutive_api_errors: 0,
            has_modifying_tool_use: false,
            has_modifying_tool_use_since_user: false,
            first_user_message: None,
            last_user_message: None,
        };
        let sub_agent = MockSubAgent::new();

        let result =
            check_interactive_question(&transcript_info, &sub_agent, &StopHookConfig::default())
                .unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_check_interactive_question_continue_question() {
        use chrono::{Duration, Utc};
        let transcript_info = TranscriptInfo {
            last_assistant_output: Some("Should I continue?".to_string()),
            last_user_message_time: Some(Utc::now() - Duration::minutes(1)),
            has_api_error: false,
            consecutive_api_errors: 0,
            has_modifying_tool_use: false,
            has_modifying_tool_use_since_user: false,
            first_user_message: None,
            last_user_message: None,
        };
        let sub_agent = MockSubAgent::new();

        let result =
            check_interactive_question(&transcript_info, &sub_agent, &StopHookConfig::default())
                .unwrap();
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
            has_api_error: false,
            consecutive_api_errors: 0,
            has_modifying_tool_use: false,
            has_modifying_tool_use_since_user: false,
            first_user_message: None,
            last_user_message: None,
        };
        let mut sub_agent = MockSubAgent::new();
        sub_agent.expect_question_decision(SubAgentDecision::AllowStop(Some(
            "User preference needed".to_string(),
        )));

        let result =
            check_interactive_question(&transcript_info, &sub_agent, &StopHookConfig::default())
                .unwrap();
        assert!(result.is_some());
        let result = result.unwrap();
        assert!(result.allow_stop);
        assert!(result
            .messages
            .iter()
            .any(|m| m.contains("User Interaction") || m.contains("asking")));
    }

    #[test]
    fn test_check_interactive_question_subagent_answer() {
        use crate::traits::SubAgentDecision;
        use chrono::{Duration, Utc};

        let transcript_info = TranscriptInfo {
            last_assistant_output: Some("Which approach should I use?".to_string()),
            last_user_message_time: Some(Utc::now() - Duration::minutes(1)),
            has_api_error: false,
            consecutive_api_errors: 0,
            has_modifying_tool_use: false,
            has_modifying_tool_use_since_user: false,
            first_user_message: None,
            last_user_message: None,
        };
        let mut sub_agent = MockSubAgent::new();
        sub_agent.expect_question_decision(SubAgentDecision::Answer("Use approach A".to_string()));

        let result =
            check_interactive_question(&transcript_info, &sub_agent, &StopHookConfig::default())
                .unwrap();
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
            has_api_error: false,
            consecutive_api_errors: 0,
            has_modifying_tool_use: false,
            has_modifying_tool_use_since_user: false,
            first_user_message: None,
            last_user_message: None,
        };
        let mut sub_agent = MockSubAgent::new();
        sub_agent.expect_question_decision(SubAgentDecision::Continue);

        let result =
            check_interactive_question(&transcript_info, &sub_agent, &StopHookConfig::default())
                .unwrap();
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
        let file_list =
            CommandOutput { exit_code: 0, stdout: "file.rs\n".to_string(), stderr: String::new() };
        let empty_success =
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() };
        let zero_commits =
            CommandOutput { exit_code: 0, stdout: "0\n".to_string(), stderr: String::new() };

        // Fast path check - returns changes so we don't exit early
        runner.expect("git", &["diff", "--stat"], has_changes);
        runner.expect("git", &["diff", "--name-only"], file_list);
        runner.expect("git", &["diff", "--cached", "--stat"], empty_success.clone());
        runner.expect("git", &["ls-files", "--others", "--exclude-standard"], empty_success);
        runner.expect("git", &["rev-list", "--count", "@{upstream}..HEAD"], zero_commits);

        runner
    }

    #[test]
    fn test_bypass_problem_phrase_enters_problem_mode() {
        use tempfile::TempDir;

        let transcript_file = create_transcript_with_output(PROBLEM_NEEDS_USER);
        let runner = mock_with_uncommitted_for_bypass();
        let sub_agent = MockSubAgent::new();

        let dir = TempDir::new().unwrap();
        let input = crate::hooks::HookInput {
            transcript_path: Some(transcript_file.path().to_string_lossy().to_string()),
            ..Default::default()
        };
        let config = StopHookConfig {
            git_repo: true,
            base_dir: Some(dir.path().to_path_buf()),
            ..Default::default()
        };

        let result = run_stop_hook(&input, &config, &runner, &sub_agent).unwrap();
        // Problem phrase now blocks and enters problem mode
        assert!(!result.allow_stop);
        assert!(result.messages.iter().any(|m| m.contains("Problem Mode")));
        // Verify problem mode marker was created
        assert!(crate::session::is_problem_mode_active(dir.path()));
    }

    #[test]
    fn test_bypass_human_input_phrase_allows_exit() {
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let transcript_file = create_transcript_with_output(HUMAN_INPUT_REQUIRED);
        let runner = mock_with_uncommitted_for_bypass();
        let sub_agent = MockSubAgent::new();
        let input = crate::hooks::HookInput {
            transcript_path: Some(transcript_file.path().to_string_lossy().to_string()),
            ..Default::default()
        };
        let config =
            StopHookConfig { base_dir: Some(dir.path().to_path_buf()), ..Default::default() };

        let result = run_stop_hook(&input, &config, &runner, &sub_agent).unwrap();
        assert!(result.allow_stop);
    }

    #[test]
    fn test_bypass_human_input_blocked_with_open_tasks_shows_suggestion() {
        use crate::paths;
        use crate::tasks::{Priority, SqliteTaskStore, TaskStore};
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // Create task database at the correct path
        let db_path = paths::project_db_path(base);
        std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();
        let store = SqliteTaskStore::new(&db_path).unwrap();
        store.create_task("Fix important bug", "Description", Priority::High).unwrap();

        let transcript_file = create_transcript_with_output(HUMAN_INPUT_REQUIRED);

        let mut runner = MockCommandRunner::new();

        let has_changes = CommandOutput {
            exit_code: 0,
            stdout: " file.rs | 10 ++++++++++\n".to_string(),
            stderr: String::new(),
        };
        let file_list =
            CommandOutput { exit_code: 0, stdout: "file.rs\n".to_string(), stderr: String::new() };
        let empty_success =
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() };
        let zero_commits =
            CommandOutput { exit_code: 0, stdout: "0\n".to_string(), stderr: String::new() };

        // check_uncommitted_changes (fast path) - returns changes so we continue
        runner.expect("git", &["diff", "--stat"], has_changes);
        runner.expect("git", &["diff", "--name-only"], file_list);
        runner.expect("git", &["diff", "--cached", "--stat"], empty_success.clone());
        runner.expect("git", &["ls-files", "--others", "--exclude-standard"], empty_success);
        runner.expect("git", &["rev-list", "--count", "@{upstream}..HEAD"], zero_commits);

        let sub_agent = MockSubAgent::new();
        let input = crate::hooks::HookInput {
            transcript_path: Some(transcript_file.path().to_string_lossy().to_string()),
            ..Default::default()
        };
        let config = StopHookConfig {
            git_repo: true,
            base_dir: Some(base.to_path_buf()),
            ..Default::default()
        };

        let result = run_stop_hook(&input, &config, &runner, &sub_agent).unwrap();
        assert!(!result.allow_stop);
        assert!(result.messages.iter().any(|m| m.contains("Exit Phrase Rejected")));
        assert!(result.messages.iter().any(|m| m.contains("1 task(s) ready to work on")));
        // Check for task suggestion
        assert!(
            result.messages.iter().any(|m| m.contains("Suggestion: Work on task")),
            "Expected task suggestion in messages: {:?}",
            result.messages
        );
        assert!(
            result.messages.iter().any(|m| m.contains("Fix important bug")),
            "Expected task title in suggestion: {:?}",
            result.messages
        );
    }

    #[test]
    fn test_quality_check_passes() {
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let mut runner = MockCommandRunner::new();

        let empty_success =
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() };
        let zero_commits =
            CommandOutput { exit_code: 0, stdout: "0\n".to_string(), stderr: String::new() };
        let quality_pass = CommandOutput {
            exit_code: 0,
            stdout: "all checks pass\n".to_string(),
            stderr: String::new(),
        };

        // check_uncommitted_changes - clean repo (no changes, not ahead)
        runner.expect("git", &["diff", "--stat"], empty_success.clone());
        runner.expect("git", &["diff", "--cached", "--stat"], empty_success.clone());
        runner.expect("git", &["ls-files", "--others", "--exclude-standard"], empty_success);
        runner.expect("git", &["rev-list", "--count", "@{upstream}..HEAD"], zero_commits);

        // Quality check runs (clean repo doesn't skip it anymore)
        runner.expect("sh", &["-c", "just check"], quality_pass);

        let sub_agent = MockSubAgent::new();
        let input = crate::hooks::HookInput::default();
        let config = StopHookConfig {
            git_repo: true,
            quality_check_enabled: true,
            quality_check_command: Some("just check".to_string()),
            base_dir: Some(dir.path().to_path_buf()),
            ..Default::default()
        };

        // Quality checks run and pass, stop is allowed
        let result = run_stop_hook(&input, &config, &runner, &sub_agent).unwrap();
        assert!(result.allow_stop);
    }

    #[test]
    fn test_quality_check_fails() {
        use tempfile::TempDir;

        // Test quality check failure
        let dir = TempDir::new().unwrap();
        let mut runner = MockCommandRunner::new();

        let empty_success =
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() };
        let zero_commits =
            CommandOutput { exit_code: 0, stdout: "0\n".to_string(), stderr: String::new() };
        let quality_fail = CommandOutput {
            exit_code: 1,
            stdout: String::new(),
            stderr: "lint failed\n".to_string(),
        };

        // check_uncommitted_changes - clean repo
        runner.expect("git", &["diff", "--stat"], empty_success.clone());
        runner.expect("git", &["diff", "--cached", "--stat"], empty_success.clone());
        runner.expect("git", &["ls-files", "--others", "--exclude-standard"], empty_success);
        runner.expect("git", &["rev-list", "--count", "@{upstream}..HEAD"], zero_commits);

        // Quality check fails
        runner.expect("sh", &["-c", "just check"], quality_fail);

        let sub_agent = MockSubAgent::new();
        let input = crate::hooks::HookInput::default();
        let config = StopHookConfig {
            git_repo: true,
            quality_check_enabled: true,
            quality_check_command: Some("just check".to_string()),
            base_dir: Some(dir.path().to_path_buf()),
            explain_stops: false,
            ..Default::default()
        };

        let result = run_stop_hook(&input, &config, &runner, &sub_agent).unwrap();
        assert!(!result.allow_stop);
        assert!(result.messages.iter().any(|m| m.contains("Quality")));
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
        let file_list =
            CommandOutput { exit_code: 0, stdout: "file.rs\n".to_string(), stderr: String::new() };
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
        runner.expect("git", &["diff", "--name-only"], file_list.clone());
        runner.expect("git", &["diff", "--cached", "--stat"], empty_success.clone());
        runner.expect(
            "git",
            &["ls-files", "--others", "--exclude-standard"],
            many_untracked.clone(),
        );
        runner.expect("git", &["rev-list", "--count", "@{upstream}..HEAD"], zero_commits.clone());

        // Second check_uncommitted_changes (main check)
        runner.expect("git", &["diff", "--stat"], has_changes);
        runner.expect("git", &["diff", "--name-only"], file_list);
        runner.expect("git", &["diff", "--cached", "--stat"], empty_success);
        runner.expect("git", &["ls-files", "--others", "--exclude-standard"], many_untracked);
        runner.expect("git", &["rev-list", "--count", "@{upstream}..HEAD"], zero_commits);

        let dir = TempDir::new().unwrap();
        let sub_agent = MockSubAgent::new();
        let input = crate::hooks::HookInput::default();
        let config = StopHookConfig {
            git_repo: true,
            base_dir: Some(dir.path().to_path_buf()),
            ..Default::default()
        };

        let result = run_stop_hook(&input, &config, &runner, &sub_agent).unwrap();
        assert!(!result.allow_stop);
        // Should show "... and 5 more" for the extra files beyond 10
        assert!(result.messages.iter().any(|m| m.contains("... and 5 more")));
    }

    #[test]
    fn test_run_stop_hook_api_error_loop_allows_exit() {
        use std::io::Write;

        // Create a transcript with multiple consecutive API errors
        let mut transcript_file = tempfile::NamedTempFile::new().unwrap();
        let error_entry1 = serde_json::json!({
            "type": "assistant",
            "isApiErrorMessage": true,
            "message": {
                "content": [{"type": "text", "text": "API Error: 400"}]
            }
        });
        let error_entry2 = serde_json::json!({
            "type": "assistant",
            "isApiErrorMessage": true,
            "message": {
                "content": [{"type": "text", "text": "API Error: 400"}]
            }
        });
        writeln!(transcript_file, "{}", serde_json::to_string(&error_entry1).unwrap()).unwrap();
        writeln!(transcript_file, "{}", serde_json::to_string(&error_entry2).unwrap()).unwrap();

        // Use mock runner with uncommitted changes (would normally block)
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

        // These expectations won't be used because we bail out early
        runner.expect("git", &["diff", "--stat"], has_changes);
        runner.expect("git", &["diff", "--cached", "--stat"], empty_success.clone());
        runner.expect("git", &["ls-files", "--others", "--exclude-standard"], empty_success);
        runner.expect("git", &["rev-list", "--count", "@{upstream}..HEAD"], zero_commits);

        let dir = TempDir::new().unwrap();
        let sub_agent = MockSubAgent::new();
        let input = crate::hooks::HookInput {
            transcript_path: Some(transcript_file.path().to_string_lossy().to_string()),
            ..Default::default()
        };
        let config = StopHookConfig {
            git_repo: true,
            base_dir: Some(dir.path().to_path_buf()),
            ..Default::default()
        };

        let result = run_stop_hook(&input, &config, &runner, &sub_agent).unwrap();
        // Should allow stop despite uncommitted changes due to API error loop
        assert!(result.allow_stop);
        assert!(result.messages.iter().any(|m| m.contains("API Error Loop")));
    }

    #[test]
    fn test_run_stop_hook_single_api_error_allows_stop() {
        use std::io::Write;

        // Create a transcript with a single API error (meets threshold of 1)
        let mut transcript_file = tempfile::NamedTempFile::new().unwrap();
        let error_entry = serde_json::json!({
            "type": "assistant",
            "isApiErrorMessage": true,
            "message": {
                "content": [{"type": "text", "text": "API Error: 400"}]
            }
        });
        writeln!(transcript_file, "{}", serde_json::to_string(&error_entry).unwrap()).unwrap();

        let dir = TempDir::new().unwrap();
        let runner = MockCommandRunner::new();
        let sub_agent = MockSubAgent::new();
        let input = crate::hooks::HookInput {
            transcript_path: Some(transcript_file.path().to_string_lossy().to_string()),
            ..Default::default()
        };
        let config = StopHookConfig {
            git_repo: true,
            base_dir: Some(dir.path().to_path_buf()),
            ..Default::default()
        };

        let result = run_stop_hook(&input, &config, &runner, &sub_agent).unwrap();
        // Single API error SHOULD allow stop (threshold is 1)
        assert!(result.allow_stop);
        assert!(result.messages.iter().any(|m| m.contains("API error")));
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
        let file_list =
            CommandOutput { exit_code: 0, stdout: "file.rs\n".to_string(), stderr: String::new() };
        let empty_success =
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() };
        let zero_commits =
            CommandOutput { exit_code: 0, stdout: "0\n".to_string(), stderr: String::new() };

        // First check_uncommitted_changes (fast path check)
        runner.expect("git", &["diff", "--stat"], has_changes.clone());
        runner.expect("git", &["diff", "--name-only"], file_list.clone());
        runner.expect("git", &["diff", "--cached", "--stat"], empty_success.clone());
        runner.expect(
            "git",
            &["ls-files", "--others", "--exclude-standard"],
            empty_success.clone(),
        );
        runner.expect("git", &["rev-list", "--count", "@{upstream}..HEAD"], zero_commits.clone());

        // Second check_uncommitted_changes (main check)
        runner.expect("git", &["diff", "--stat"], has_changes);
        runner.expect("git", &["diff", "--name-only"], file_list);
        runner.expect("git", &["diff", "--cached", "--stat"], empty_success.clone());
        runner.expect("git", &["ls-files", "--others", "--exclude-standard"], empty_success);
        runner.expect("git", &["rev-list", "--count", "@{upstream}..HEAD"], zero_commits);

        let dir = TempDir::new().unwrap();
        let sub_agent = MockSubAgent::new();
        let input = crate::hooks::HookInput::default();
        let config = StopHookConfig {
            git_repo: true,
            require_push: true,
            base_dir: Some(dir.path().to_path_buf()),
            ..Default::default()
        };

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

        let dir = TempDir::new().unwrap();
        let mut sub_agent = MockSubAgent::new();
        sub_agent.expect_question_decision(SubAgentDecision::AllowStop(Some(
            "User preference needed".to_string(),
        )));

        let input = crate::hooks::HookInput {
            transcript_path: Some(transcript_file.path().to_string_lossy().to_string()),
            ..Default::default()
        };
        // require_push = false so we don't block on unpushed commits
        let config = StopHookConfig {
            git_repo: true,
            require_push: false,
            base_dir: Some(dir.path().to_path_buf()),
            ..Default::default()
        };

        let result = run_stop_hook(&input, &config, &runner, &sub_agent).unwrap();
        assert!(result.allow_stop);
        assert!(result
            .messages
            .iter()
            .any(|m| m.contains("User Interaction") || m.contains("asking")));
    }

    #[test]
    fn test_run_stop_hook_quality_check_with_uncommitted_changes() {
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();

        // Test quality check running when there are uncommitted changes
        let mut runner = MockCommandRunner::new();
        let has_changes = CommandOutput {
            exit_code: 0,
            stdout: " file.rs | 10 ++++++++++\n".to_string(),
            stderr: String::new(),
        };
        let file_list =
            CommandOutput { exit_code: 0, stdout: "file.rs\n".to_string(), stderr: String::new() };
        let empty_success =
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() };
        let zero_commits =
            CommandOutput { exit_code: 0, stdout: "0\n".to_string(), stderr: String::new() };
        let quality_pass = CommandOutput {
            exit_code: 0,
            stdout: "All checks passed\n".to_string(),
            stderr: String::new(),
        };

        // check_uncommitted_changes - detects changes
        runner.expect("git", &["diff", "--stat"], has_changes);
        runner.expect("git", &["diff", "--name-only"], file_list);
        runner.expect("git", &["diff", "--cached", "--stat"], empty_success.clone());
        runner.expect("git", &["ls-files", "--others", "--exclude-standard"], empty_success);
        runner.expect("git", &["rev-list", "--count", "@{upstream}..HEAD"], zero_commits);

        // Quality check passes (runs inside handle_uncommitted_changes)
        runner.expect("sh", &["-c", "just check"], quality_pass);

        let sub_agent = MockSubAgent::new();
        let input = crate::hooks::HookInput::default();
        let config = StopHookConfig {
            git_repo: true,
            quality_check_enabled: true,
            quality_check_command: Some("just check".to_string()),
            base_dir: Some(dir.path().to_path_buf()),
            ..Default::default()
        };

        let result = run_stop_hook(&input, &config, &runner, &sub_agent).unwrap();
        assert!(!result.allow_stop);
        // Should have run quality checks
        assert!(result.messages.iter().any(|m| m.contains("Running Quality Checks")));
    }

    #[test]
    fn test_problem_mode_exit() {
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // Enter problem mode first
        crate::session::enter_problem_mode(base).unwrap();

        let runner = MockCommandRunner::new();
        let sub_agent = MockSubAgent::new();

        let config = StopHookConfig {
            git_repo: false,
            base_dir: Some(base.to_path_buf()),
            ..Default::default()
        };

        let input = crate::hooks::HookInput::default();

        let result = run_stop_hook(&input, &config, &runner, &sub_agent).unwrap();
        assert!(result.allow_stop);
        assert!(result.messages.iter().any(|m| m.contains("Problem Mode Exit")));
    }

    #[test]
    fn test_simple_reflection_prompts_on_modifying_tool_use() {
        use std::io::Write;
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // Create a transcript with modifying tool use
        let transcript_path = base.join("transcript.jsonl");
        {
            let mut file = std::fs::File::create(&transcript_path).unwrap();
            writeln!(
                file,
                r#"{{"type":"assistant","message":{{"content":[{{"type":"tool_use","name":"Write","id":"123"}}]}}}}"#
            )
            .unwrap();
        }

        let runner = MockCommandRunner::new();
        let sub_agent = MockSubAgent::new();
        let input = crate::hooks::HookInput {
            transcript_path: Some(transcript_path.to_string_lossy().to_string()),
            ..Default::default()
        };
        let config = StopHookConfig {
            git_repo: false,
            base_dir: Some(base.to_path_buf()),
            ..Default::default()
        };

        let result = run_stop_hook(&input, &config, &runner, &sub_agent).unwrap();
        assert!(!result.allow_stop);
        assert!(result.messages.iter().any(|m| m.contains("Task Completion Check")));
        // Marker should be set
        assert!(session::has_reflect_marker(base));
    }

    #[test]
    fn test_simple_reflection_allows_second_stop() {
        use std::io::Write;
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // Set up the reflect marker (simulating first stop already happened)
        session::set_reflect_marker(base).unwrap();

        // Create a transcript without modifying tool use (just reading)
        let transcript_path = base.join("transcript.jsonl");
        {
            let mut file = std::fs::File::create(&transcript_path).unwrap();
            writeln!(
                file,
                r#"{{"type":"assistant","message":{{"content":[{{"type":"text","text":"Summary of work done."}}]}}}}"#
            )
            .unwrap();
        }

        let runner = MockCommandRunner::new();
        let sub_agent = MockSubAgent::new();
        let input = crate::hooks::HookInput {
            transcript_path: Some(transcript_path.to_string_lossy().to_string()),
            ..Default::default()
        };
        let config = StopHookConfig {
            git_repo: false,
            base_dir: Some(base.to_path_buf()),
            ..Default::default()
        };

        let result = run_stop_hook(&input, &config, &runner, &sub_agent).unwrap();
        assert!(result.allow_stop);
        // Marker should be cleared after second stop
        assert!(!session::has_reflect_marker(base));
    }

    #[test]
    fn test_simple_reflection_no_modifying_tools_no_prompt() {
        use std::io::Write;
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // Create a transcript with only read operations
        let transcript_path = base.join("transcript.jsonl");
        {
            let mut file = std::fs::File::create(&transcript_path).unwrap();
            writeln!(
                file,
                r#"{{"type":"assistant","message":{{"content":[{{"type":"tool_use","name":"Read","id":"123"}}]}}}}"#
            )
            .unwrap();
        }

        let runner = MockCommandRunner::new();
        let sub_agent = MockSubAgent::new();
        let input = crate::hooks::HookInput {
            transcript_path: Some(transcript_path.to_string_lossy().to_string()),
            ..Default::default()
        };
        let config = StopHookConfig {
            git_repo: false,
            base_dir: Some(base.to_path_buf()),
            ..Default::default()
        };

        let result = run_stop_hook(&input, &config, &runner, &sub_agent).unwrap();
        assert!(result.allow_stop);
        // No marker should be set
        assert!(!session::has_reflect_marker(base));
    }

    #[test]
    fn test_simple_reflection_skipped_when_agent_asks_question() {
        use std::io::Write;
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // Create a transcript with modifying tools BUT ending with a question
        let transcript_path = base.join("transcript.jsonl");
        {
            let mut file = std::fs::File::create(&transcript_path).unwrap();
            // Write tool use
            writeln!(
                file,
                r#"{{"type":"assistant","message":{{"content":[{{"type":"tool_use","name":"Write","id":"123"}}]}}}}"#
            )
            .unwrap();
            // Write tool result
            writeln!(
                file,
                r#"{{"type":"user","message":{{"content":[{{"type":"tool_result","tool_use_id":"123","content":"ok"}}]}}}}"#
            )
            .unwrap();
            // Write assistant message with a question
            writeln!(
                file,
                r#"{{"type":"assistant","message":{{"content":[{{"type":"text","text":"I've completed the changes. Would you like me to continue?"}}]}}}}"#
            )
            .unwrap();
        }

        let runner = MockCommandRunner::new();
        let sub_agent = MockSubAgent::new();
        let input = crate::hooks::HookInput {
            transcript_path: Some(transcript_path.to_string_lossy().to_string()),
            ..Default::default()
        };
        let config = StopHookConfig {
            git_repo: false,
            base_dir: Some(base.to_path_buf()),
            ..Default::default()
        };

        let result = run_stop_hook(&input, &config, &runner, &sub_agent).unwrap();
        // Should allow stop because agent asked a question
        assert!(result.allow_stop);
        // No reflection marker should be set
        assert!(!session::has_reflect_marker(base));
    }

    #[test]
    fn test_check_commit_push_question_commit() {
        assert_eq!(
            check_commit_push_question("Would you like me to commit these changes?"),
            Some("Yes, please commit these changes.".to_string())
        );
        assert_eq!(
            check_commit_push_question(
                "Here's the summary.\n\nWould you like me to commit these changes?"
            ),
            Some("Yes, please commit these changes.".to_string())
        );
        assert_eq!(
            check_commit_push_question("Should I commit these changes?"),
            Some("Yes, please commit these changes.".to_string())
        );
    }

    #[test]
    fn test_check_commit_push_question_push() {
        assert_eq!(
            check_commit_push_question("Would you like me to push these changes?"),
            Some("Yes, please push.".to_string())
        );
        assert_eq!(
            check_commit_push_question("Should I push?"),
            Some("Yes, please push.".to_string())
        );
    }

    #[test]
    fn test_check_commit_push_question_both() {
        assert_eq!(
            check_commit_push_question("Would you like me to commit and push?"),
            Some("Yes, please commit and push.".to_string())
        );
        assert_eq!(
            check_commit_push_question("Should I commit and push?"),
            Some("Yes, please commit and push.".to_string())
        );
    }

    #[test]
    fn test_check_commit_push_question_none() {
        assert_eq!(check_commit_push_question("Here's the summary."), None);
        assert_eq!(check_commit_push_question("Done with the changes."), None);
        assert_eq!(check_commit_push_question("What would you like me to do next?"), None);
        // Not at end of message
        assert_eq!(check_commit_push_question("Would you like me to commit? Let me know."), None);
    }

    #[test]
    fn test_run_stop_hook_auto_confirms_commit_question() {
        use std::io::Write;

        let dir = tempfile::TempDir::new().unwrap();

        // Create a transcript with the commit question
        let transcript_path = dir.path().join("transcript.jsonl");
        {
            let mut file = std::fs::File::create(&transcript_path).unwrap();
            writeln!(
                file,
                r#"{{"type":"assistant","message":{{"content":[{{"type":"text","text":"Changes committed.\n\nWould you like me to commit these changes?"}}]}}}}"#
            )
            .unwrap();
        }

        let runner = MockCommandRunner::new();
        let sub_agent = MockSubAgent::new();
        let input = crate::hooks::HookInput {
            transcript_path: Some(transcript_path.to_string_lossy().to_string()),
            ..Default::default()
        };
        let config = StopHookConfig {
            git_repo: true,
            base_dir: Some(dir.path().to_path_buf()),
            ..Default::default()
        };

        let result = run_stop_hook(&input, &config, &runner, &sub_agent).unwrap();
        assert!(!result.allow_stop); // Block, but with inject
        assert_eq!(result.inject_response, Some("Yes, please commit these changes.".to_string()));
    }

    #[test]
    fn test_validation_blocks_when_needed_and_fails() {
        let dir = tempfile::TempDir::new().unwrap();
        let base = dir.path();

        // Set needs_validation marker
        session::set_needs_validation(base).unwrap();

        // Set up mock runner that returns failure for the check command
        let mut runner = MockCommandRunner::new();
        runner.expect(
            "sh",
            &["-c", "just check"],
            CommandOutput {
                exit_code: 1,
                stdout: "Error: tests failed\n".to_string(),
                stderr: String::new(),
            },
        );

        let sub_agent = MockSubAgent::new();
        let input = crate::hooks::HookInput::default();
        let config = StopHookConfig {
            quality_check_command: Some("just check".to_string()),
            base_dir: Some(base.to_path_buf()),
            ..Default::default()
        };

        let result = run_stop_hook(&input, &config, &runner, &sub_agent).unwrap();

        // Should block
        assert!(!result.allow_stop);
        assert!(result.messages.iter().any(|m| m.contains("Validation Failed")));
        // Marker should still be set
        assert!(session::needs_validation(base));
    }

    #[test]
    fn test_validation_passes_and_clears_marker() {
        let dir = tempfile::TempDir::new().unwrap();
        let base = dir.path();

        // Set needs_validation marker
        session::set_needs_validation(base).unwrap();

        // Set up mock runner that returns success for the check command
        let mut runner = MockCommandRunner::new();
        runner.expect(
            "sh",
            &["-c", "just check"],
            CommandOutput {
                exit_code: 0,
                stdout: "All checks passed\n".to_string(),
                stderr: String::new(),
            },
        );
        // After validation passes, it will check git status
        runner.expect(
            "git",
            &["diff", "--stat"],
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() },
        );
        runner.expect(
            "git",
            &["diff", "--cached", "--stat"],
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() },
        );
        runner.expect(
            "git",
            &["ls-files", "--others", "--exclude-standard"],
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() },
        );
        runner.expect(
            "git",
            &["rev-list", "--count", "@{upstream}..HEAD"],
            CommandOutput { exit_code: 0, stdout: "0\n".to_string(), stderr: String::new() },
        );

        let sub_agent = MockSubAgent::new();
        let input = crate::hooks::HookInput::default();
        let config = StopHookConfig {
            git_repo: true,
            quality_check_command: Some("just check".to_string()),
            base_dir: Some(base.to_path_buf()),
            ..Default::default()
        };

        let result = run_stop_hook(&input, &config, &runner, &sub_agent).unwrap();

        // Should allow (validation passed and git is clean)
        assert!(result.allow_stop);
        // Marker should be cleared
        assert!(!session::needs_validation(base));
    }

    #[test]
    fn test_no_validation_when_marker_not_set() {
        let dir = tempfile::TempDir::new().unwrap();
        let base = dir.path();

        // Don't set needs_validation marker
        assert!(!session::needs_validation(base));

        // Set up mock runner - no check command should be called
        let mut runner = MockCommandRunner::new();
        // Only git status checks
        runner.expect(
            "git",
            &["diff", "--stat"],
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() },
        );
        runner.expect(
            "git",
            &["diff", "--cached", "--stat"],
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() },
        );
        runner.expect(
            "git",
            &["ls-files", "--others", "--exclude-standard"],
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() },
        );
        runner.expect(
            "git",
            &["rev-list", "--count", "@{upstream}..HEAD"],
            CommandOutput { exit_code: 0, stdout: "0\n".to_string(), stderr: String::new() },
        );

        let sub_agent = MockSubAgent::new();
        let input = crate::hooks::HookInput::default();
        let config = StopHookConfig {
            git_repo: true,
            quality_check_command: Some("just check".to_string()),
            base_dir: Some(base.to_path_buf()),
            ..Default::default()
        };

        let result = run_stop_hook(&input, &config, &runner, &sub_agent).unwrap();

        // Should allow (no validation needed)
        assert!(result.allow_stop);
    }

    #[test]
    fn test_validation_shows_stderr_on_failure() {
        let dir = tempfile::TempDir::new().unwrap();
        let base = dir.path();

        session::set_needs_validation(base).unwrap();

        let mut runner = MockCommandRunner::new();
        runner.expect(
            "sh",
            &["-c", "just check"],
            CommandOutput {
                exit_code: 1,
                stdout: String::new(),
                stderr: "error: compilation failed\n".to_string(),
            },
        );

        let sub_agent = MockSubAgent::new();
        let input = crate::hooks::HookInput::default();
        let config = StopHookConfig {
            quality_check_command: Some("just check".to_string()),
            base_dir: Some(base.to_path_buf()),
            ..Default::default()
        };

        let result = run_stop_hook(&input, &config, &runner, &sub_agent).unwrap();

        assert!(!result.allow_stop);
        assert!(result.messages.iter().any(|m| m.contains("stderr")));
        assert!(result.messages.iter().any(|m| m.contains("compilation failed")));
    }

    #[test]
    fn test_simple_question_fast_path_allows_stop() {
        // When the first user message is a simple question (single line ending with ?)
        // and no modifying tools were used, should allow immediate stop
        let dir = tempfile::TempDir::new().unwrap();
        let base = dir.path();

        let runner = MockCommandRunner::new();
        let sub_agent = MockSubAgent::new();

        // Create input with transcript path
        let transcript_content = r#"{"type": "user", "message": {"role": "user", "content": "What does this function do?"}}
{"type": "assistant", "message": {"content": [{"type": "tool_use", "name": "Read", "id": "1"}]}}
{"type": "assistant", "message": {"content": [{"type": "text", "text": "This function calculates the sum."}]}}
"#;
        let transcript_file = base.join("transcript.jsonl");
        std::fs::write(&transcript_file, transcript_content).unwrap();

        let input = crate::hooks::HookInput {
            transcript_path: Some(transcript_file.to_string_lossy().to_string()),
            ..Default::default()
        };

        let config = StopHookConfig {
            git_repo: false, // Skip git checks
            base_dir: Some(base.to_path_buf()),
            ..Default::default()
        };

        let result = run_stop_hook(&input, &config, &runner, &sub_agent).unwrap();

        // Simple question with only read tools should allow immediate stop
        assert!(result.allow_stop, "Simple question with read-only tools should allow stop");
    }

    #[test]
    fn test_simple_question_with_modifications_no_fast_path() {
        // When there are modifications, the first stop prompts reflection.
        // On second stop (with reflection marker set), it should allow.
        let dir = tempfile::TempDir::new().unwrap();
        let base = dir.path();

        // Set reflection marker (simulating second stop after reflection prompt)
        session::set_reflect_marker(base).unwrap();

        let mut runner = MockCommandRunner::new();
        // Will need git status checks
        runner.expect(
            "git",
            &["diff", "--stat"],
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() },
        );
        runner.expect(
            "git",
            &["diff", "--cached", "--stat"],
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() },
        );
        runner.expect(
            "git",
            &["ls-files", "--others", "--exclude-standard"],
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() },
        );
        runner.expect(
            "git",
            &["rev-list", "--count", "@{upstream}..HEAD"],
            CommandOutput { exit_code: 0, stdout: "0\n".to_string(), stderr: String::new() },
        );

        let sub_agent = MockSubAgent::new();

        // Create transcript with modifications (Edit tool)
        let transcript_content = r#"{"type": "user", "message": {"role": "user", "content": "What does this function do?"}}
{"type": "assistant", "message": {"content": [{"type": "tool_use", "name": "Edit", "id": "1"}]}}
{"type": "assistant", "message": {"content": [{"type": "text", "text": "I've updated the function."}]}}
"#;
        let transcript_file = base.join("transcript.jsonl");
        std::fs::write(&transcript_file, transcript_content).unwrap();

        let input = crate::hooks::HookInput {
            transcript_path: Some(transcript_file.to_string_lossy().to_string()),
            ..Default::default()
        };

        let config = StopHookConfig {
            git_repo: true,
            base_dir: Some(base.to_path_buf()),
            ..Default::default()
        };

        let result = run_stop_hook(&input, &config, &runner, &sub_agent).unwrap();

        // Should allow after reflection (git is clean, marker was set)
        assert!(result.allow_stop);
    }

    #[test]
    fn test_multiline_message_no_fast_path() {
        // Multiline messages should NOT use the fast path even without modifications
        let dir = tempfile::TempDir::new().unwrap();
        let base = dir.path();

        let mut runner = MockCommandRunner::new();
        // Will need git status checks since fast path doesn't apply
        runner.expect(
            "git",
            &["diff", "--stat"],
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() },
        );
        runner.expect(
            "git",
            &["diff", "--cached", "--stat"],
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() },
        );
        runner.expect(
            "git",
            &["ls-files", "--others", "--exclude-standard"],
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() },
        );
        runner.expect(
            "git",
            &["rev-list", "--count", "@{upstream}..HEAD"],
            CommandOutput { exit_code: 0, stdout: "0\n".to_string(), stderr: String::new() },
        );

        let sub_agent = MockSubAgent::new();

        // Create transcript with multiline question
        let transcript_content = r#"{"type": "user", "message": {"role": "user", "content": "What does this function do?\nAlso explain the parameters."}}
{"type": "assistant", "message": {"content": [{"type": "tool_use", "name": "Read", "id": "1"}]}}
{"type": "assistant", "message": {"content": [{"type": "text", "text": "It does X."}]}}
"#;
        let transcript_file = base.join("transcript.jsonl");
        std::fs::write(&transcript_file, transcript_content).unwrap();

        let input = crate::hooks::HookInput {
            transcript_path: Some(transcript_file.to_string_lossy().to_string()),
            ..Default::default()
        };

        let config = StopHookConfig {
            git_repo: true,
            base_dir: Some(base.to_path_buf()),
            ..Default::default()
        };

        let result = run_stop_hook(&input, &config, &runner, &sub_agent).unwrap();

        // Should still allow (git is clean) but via normal path
        assert!(result.allow_stop);
    }

    #[test]
    fn test_non_question_no_fast_path() {
        // Non-questions (no trailing ?) should NOT use the fast path
        let dir = tempfile::TempDir::new().unwrap();
        let base = dir.path();

        let mut runner = MockCommandRunner::new();
        // Will need git status checks since fast path doesn't apply
        runner.expect(
            "git",
            &["diff", "--stat"],
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() },
        );
        runner.expect(
            "git",
            &["diff", "--cached", "--stat"],
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() },
        );
        runner.expect(
            "git",
            &["ls-files", "--others", "--exclude-standard"],
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() },
        );
        runner.expect(
            "git",
            &["rev-list", "--count", "@{upstream}..HEAD"],
            CommandOutput { exit_code: 0, stdout: "0\n".to_string(), stderr: String::new() },
        );

        let sub_agent = MockSubAgent::new();

        // Create transcript with command, not question
        let transcript_content = r#"{"type": "user", "message": {"role": "user", "content": "Read the README file"}}
{"type": "assistant", "message": {"content": [{"type": "tool_use", "name": "Read", "id": "1"}]}}
{"type": "assistant", "message": {"content": [{"type": "text", "text": "Here is the README content."}]}}
"#;
        let transcript_file = base.join("transcript.jsonl");
        std::fs::write(&transcript_file, transcript_content).unwrap();

        let input = crate::hooks::HookInput {
            transcript_path: Some(transcript_file.to_string_lossy().to_string()),
            ..Default::default()
        };

        let config = StopHookConfig {
            git_repo: true,
            base_dir: Some(base.to_path_buf()),
            ..Default::default()
        };

        let result = run_stop_hook(&input, &config, &runner, &sub_agent).unwrap();

        // Should still allow (git is clean) but via normal path
        assert!(result.allow_stop);
    }

    #[test]
    fn test_bypass_human_input_blocked_with_question_blocked_tasks_first_time() {
        use crate::paths;
        use crate::tasks::{Priority, SqliteTaskStore, TaskStore};
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // Create task database at the correct path
        let db_path = paths::project_db_path(base);
        std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();
        let store = SqliteTaskStore::new(&db_path).unwrap();

        // Create a task blocked by a question
        let task = store.create_task("Implement feature", "Description", Priority::High).unwrap();
        let question = store.create_question("What should the API return?").unwrap();
        store.link_task_to_question(&task.id, &question.id).unwrap();

        let transcript_file = create_transcript_with_output(HUMAN_INPUT_REQUIRED);

        let runner = MockCommandRunner::new();

        let sub_agent = MockSubAgent::new();
        let input = crate::hooks::HookInput {
            transcript_path: Some(transcript_file.path().to_string_lossy().to_string()),
            ..Default::default()
        };
        let config = StopHookConfig {
            git_repo: false,
            base_dir: Some(base.to_path_buf()),
            ..Default::default()
        };

        // First attempt - should block and ask for reflection
        let result = run_stop_hook(&input, &config, &runner, &sub_agent).unwrap();
        assert!(!result.allow_stop);
        assert!(
            result.messages.iter().any(|m| m.contains("Questions Require Reflection")),
            "Expected reflection message in: {:?}",
            result.messages
        );
        assert!(
            result.messages.iter().any(|m| m.contains("What should the API return?")),
            "Expected question text in messages: {:?}",
            result.messages
        );
        assert!(
            result.messages.iter().any(|m| m.contains("reflect on these questions")),
            "Expected reflection instruction in messages: {:?}",
            result.messages
        );
    }

    #[test]
    fn test_bypass_human_input_allowed_with_question_blocked_tasks_second_time() {
        use crate::paths;
        use crate::tasks::{Priority, SqliteTaskStore, TaskStore};
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // Create task database at the correct path
        let db_path = paths::project_db_path(base);
        std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();
        let store = SqliteTaskStore::new(&db_path).unwrap();

        // Create a task blocked by a question
        let task = store.create_task("Implement feature", "Description", Priority::High).unwrap();
        let question = store.create_question("What should the API return?").unwrap();
        store.link_task_to_question(&task.id, &question.id).unwrap();

        // Set the marker to simulate second attempt
        session::set_questions_shown_marker(base).unwrap();

        let transcript_file = create_transcript_with_output(HUMAN_INPUT_REQUIRED);

        let runner = MockCommandRunner::new();

        let sub_agent = MockSubAgent::new();
        let input = crate::hooks::HookInput {
            transcript_path: Some(transcript_file.path().to_string_lossy().to_string()),
            ..Default::default()
        };
        let config = StopHookConfig {
            git_repo: false,
            base_dir: Some(base.to_path_buf()),
            ..Default::default()
        };

        // Second attempt - should allow and present questions
        let result = run_stop_hook(&input, &config, &runner, &sub_agent).unwrap();
        assert!(result.allow_stop);
        assert!(
            result.messages.iter().any(|m| m.contains("Questions for User")),
            "Expected user questions in messages: {:?}",
            result.messages
        );
        assert!(
            result.messages.iter().any(|m| m.contains("What should the API return?")),
            "Expected question text in messages: {:?}",
            result.messages
        );

        // Marker should be cleared
        assert!(!session::has_questions_shown_marker(base));
    }

    #[test]
    fn test_question_blocked_tasks_deduplicates_questions() {
        use crate::paths;
        use crate::tasks::{Priority, SqliteTaskStore, TaskStore};
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // Create task database at the correct path
        let db_path = paths::project_db_path(base);
        std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();
        let store = SqliteTaskStore::new(&db_path).unwrap();

        // Create multiple tasks blocked by the same question
        let task1 = store.create_task("Task 1", "", Priority::High).unwrap();
        let task2 = store.create_task("Task 2", "", Priority::High).unwrap();
        let question = store.create_question("Shared question?").unwrap();
        store.link_task_to_question(&task1.id, &question.id).unwrap();
        store.link_task_to_question(&task2.id, &question.id).unwrap();

        // Set the marker to simulate second attempt
        session::set_questions_shown_marker(base).unwrap();

        let transcript_file = create_transcript_with_output(HUMAN_INPUT_REQUIRED);

        let runner = MockCommandRunner::new();

        let sub_agent = MockSubAgent::new();
        let input = crate::hooks::HookInput {
            transcript_path: Some(transcript_file.path().to_string_lossy().to_string()),
            ..Default::default()
        };
        let config = StopHookConfig {
            git_repo: false,
            base_dir: Some(base.to_path_buf()),
            ..Default::default()
        };

        let result = run_stop_hook(&input, &config, &runner, &sub_agent).unwrap();
        assert!(result.allow_stop);

        // The question should only appear once (deduplicated)
        let question_count =
            result.messages.iter().filter(|m| m.contains("Shared question?")).count();
        assert_eq!(question_count, 1, "Question should only appear once: {:?}", result.messages);
    }

    #[test]
    fn test_followup_question_fast_path_allows_stop() {
        // When the user asks a follow-up question after a work session,
        // if no modifications were made since the last user message, allow stop
        let dir = tempfile::TempDir::new().unwrap();
        let base = dir.path();

        let runner = MockCommandRunner::new();
        let sub_agent = MockSubAgent::new();

        // Transcript: work session with edits, then a simple follow-up question
        let transcript_content = r#"{"type": "user", "timestamp": "2024-01-01T12:00:00Z", "message": {"role": "user", "content": "Create a new file"}}
{"type": "assistant", "message": {"content": [{"type": "tool_use", "name": "Write", "id": "1"}]}}
{"type": "assistant", "message": {"content": [{"type": "text", "text": "Created the file."}]}}
{"type": "user", "timestamp": "2024-01-01T12:05:00Z", "message": {"role": "user", "content": "What's the filename?"}}
{"type": "assistant", "message": {"content": [{"type": "text", "text": "The filename is test.txt"}]}}
"#;
        let transcript_file = base.join("transcript.jsonl");
        std::fs::write(&transcript_file, transcript_content).unwrap();

        let input = crate::hooks::HookInput {
            transcript_path: Some(transcript_file.to_string_lossy().to_string()),
            ..Default::default()
        };

        let config = StopHookConfig {
            git_repo: false, // Skip git checks
            base_dir: Some(base.to_path_buf()),
            ..Default::default()
        };

        let result = run_stop_hook(&input, &config, &runner, &sub_agent).unwrap();

        // Follow-up question with no modifications since should allow stop
        assert!(
            result.allow_stop,
            "Follow-up question with no modifications since should allow stop"
        );
    }

    #[test]
    fn test_followup_question_with_modifications_no_fast_path() {
        // If modifications were made after the last user message, don't use fast path
        let dir = tempfile::TempDir::new().unwrap();
        let base = dir.path();

        // Set reflection marker since modifying tools trigger reflection on first stop
        session::set_reflect_marker(base).unwrap();

        let mut runner = MockCommandRunner::new();
        // Will need git status checks since fast path doesn't apply
        runner.expect(
            "git",
            &["diff", "--stat"],
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() },
        );
        runner.expect(
            "git",
            &["diff", "--cached", "--stat"],
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() },
        );
        runner.expect(
            "git",
            &["ls-files", "--others", "--exclude-standard"],
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() },
        );
        runner.expect(
            "git",
            &["rev-list", "--count", "@{upstream}..HEAD"],
            CommandOutput { exit_code: 0, stdout: "0\n".to_string(), stderr: String::new() },
        );

        let sub_agent = MockSubAgent::new();

        // Transcript: user asks question, then agent makes modifications while answering
        let transcript_content = r#"{"type": "user", "timestamp": "2024-01-01T12:00:00Z", "message": {"role": "user", "content": "What's in the config?"}}
{"type": "assistant", "message": {"content": [{"type": "tool_use", "name": "Read", "id": "1"}]}}
{"type": "assistant", "message": {"content": [{"type": "tool_use", "name": "Edit", "id": "2"}]}}
{"type": "assistant", "message": {"content": [{"type": "text", "text": "I fixed the config for you."}]}}
"#;
        let transcript_file = base.join("transcript.jsonl");
        std::fs::write(&transcript_file, transcript_content).unwrap();

        let input = crate::hooks::HookInput {
            transcript_path: Some(transcript_file.to_string_lossy().to_string()),
            ..Default::default()
        };

        let config = StopHookConfig {
            git_repo: true,
            base_dir: Some(base.to_path_buf()),
            ..Default::default()
        };

        let result = run_stop_hook(&input, &config, &runner, &sub_agent).unwrap();

        // Should still allow (git is clean) but via normal path
        assert!(result.allow_stop);
    }

    // ========== Requested Tasks Tests ==========

    #[test]
    fn test_check_incomplete_requested_tasks_none() {
        let dir = TempDir::new().unwrap();

        // Set up database directory
        let db_path = crate::paths::project_db_path(dir.path());
        std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();

        let config =
            StopHookConfig { base_dir: Some(dir.path().to_path_buf()), ..Default::default() };

        // No tasks - should return None
        let result = check_incomplete_requested_tasks(&config);
        assert!(result.is_none());
    }

    #[test]
    fn test_check_incomplete_requested_tasks_blocks() {
        use crate::tasks::{Priority, SqliteTaskStore, TaskStore};

        let dir = TempDir::new().unwrap();

        // Set up database directory
        let db_path = crate::paths::project_db_path(dir.path());
        std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();

        // Create a task store and request a task
        let store = SqliteTaskStore::for_project(dir.path()).unwrap();
        let task = store.create_task("Important task", "", Priority::High).unwrap();
        store.request_tasks(&[&task.id]).unwrap();

        let config =
            StopHookConfig { base_dir: Some(dir.path().to_path_buf()), ..Default::default() };

        // Should block because there's an incomplete requested task
        let result = check_incomplete_requested_tasks(&config);
        assert!(result.is_some());

        let result = result.unwrap();
        assert!(!result.allow_stop);
        assert!(result.messages.iter().any(|m| m.contains("Requested Tasks Incomplete")));
        assert!(result.messages.iter().any(|m| m.contains("Important task")));
    }

    #[test]
    fn test_check_incomplete_requested_tasks_completed_allows() {
        use crate::tasks::{Priority, SqliteTaskStore, Status, TaskStore, TaskUpdate};

        let dir = TempDir::new().unwrap();

        // Set up database directory
        let db_path = crate::paths::project_db_path(dir.path());
        std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();

        // Create a task store, request a task, and complete it
        let store = SqliteTaskStore::for_project(dir.path()).unwrap();
        let task = store.create_task("Task to complete", "", Priority::High).unwrap();
        store.request_tasks(&[&task.id]).unwrap();
        store
            .update_task(
                &task.id,
                TaskUpdate { status: Some(Status::Complete), ..Default::default() },
            )
            .unwrap();

        let config =
            StopHookConfig { base_dir: Some(dir.path().to_path_buf()), ..Default::default() };

        // Should allow because task is completed
        let result = check_incomplete_requested_tasks(&config);
        assert!(result.is_none());
    }

    #[test]
    fn test_check_incomplete_requested_tasks_blocked_on_question_allows() {
        use crate::tasks::{Priority, SqliteTaskStore, TaskStore};

        let dir = TempDir::new().unwrap();

        // Set up database directory
        let db_path = crate::paths::project_db_path(dir.path());
        std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();

        // Create a task store, request a task, and block it with a question
        let store = SqliteTaskStore::for_project(dir.path()).unwrap();
        let task = store.create_task("Task blocked by question", "", Priority::High).unwrap();
        store.request_tasks(&[&task.id]).unwrap();
        let question = store.create_question("What should I do?").unwrap();
        store.link_task_to_question(&task.id, &question.id).unwrap();

        let config =
            StopHookConfig { base_dir: Some(dir.path().to_path_buf()), ..Default::default() };

        // Should allow because task is blocked on a question
        let result = check_incomplete_requested_tasks(&config);
        assert!(result.is_none());
    }

    #[test]
    fn test_check_incomplete_requested_tasks_clears_request_mode() {
        use crate::tasks::{Priority, SqliteTaskStore, Status, TaskStore, TaskUpdate};

        let dir = TempDir::new().unwrap();

        // Set up database directory
        let db_path = crate::paths::project_db_path(dir.path());
        std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();

        // Create a task store with request mode active
        let store = SqliteTaskStore::for_project(dir.path()).unwrap();
        let task = store.create_task("Task", "", Priority::High).unwrap();
        store.request_all_open().unwrap();
        assert!(store.is_request_mode_active().unwrap());

        // Complete the task
        store
            .update_task(
                &task.id,
                TaskUpdate { status: Some(Status::Complete), ..Default::default() },
            )
            .unwrap();

        let config =
            StopHookConfig { base_dir: Some(dir.path().to_path_buf()), ..Default::default() };

        // Check should return None (allow) and clear request mode
        let result = check_incomplete_requested_tasks(&config);
        assert!(result.is_none());

        // Request mode should now be cleared
        assert!(!store.is_request_mode_active().unwrap());
    }

    #[test]
    fn test_run_stop_hook_blocks_with_incomplete_requested_tasks() {
        use crate::tasks::{Priority, SqliteTaskStore, TaskStore};

        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // Set up database directory
        let db_path = crate::paths::project_db_path(base);
        std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();

        // Create and request a task
        let store = SqliteTaskStore::for_project(base).unwrap();
        let task = store.create_task("Incomplete task", "", Priority::High).unwrap();
        store.request_tasks(&[&task.id]).unwrap();

        // Set up git to have uncommitted changes
        let runner = mock_uncommitted_changes();
        let sub_agent = MockSubAgent::new();

        // Create a transcript with modifying tool use and assistant output
        let transcript_content = r#"{"type": "user", "timestamp": "2024-01-01T12:00:00Z", "message": {"role": "user", "content": "Do the task"}}
{"type": "assistant", "message": {"content": [{"type": "tool_use", "name": "Edit", "id": "1"}]}}
{"type": "assistant", "message": {"content": [{"type": "text", "text": "I made some changes."}]}}
"#;
        let transcript_file = base.join("transcript.jsonl");
        std::fs::write(&transcript_file, transcript_content).unwrap();

        let input = crate::hooks::HookInput {
            transcript_path: Some(transcript_file.to_string_lossy().to_string()),
            ..Default::default()
        };
        let config = StopHookConfig {
            git_repo: true,
            base_dir: Some(base.to_path_buf()),
            ..Default::default()
        };

        let result = run_stop_hook(&input, &config, &runner, &sub_agent).unwrap();

        // Should block because there's an incomplete requested task
        assert!(!result.allow_stop);
        assert!(result.messages.iter().any(|m| m.contains("Requested Tasks Incomplete")));
    }

    /// Create a mock that returns no uncommitted changes but IS ahead of remote.
    /// This allows skipping the fast path at line 283 (because we're ahead)
    /// while also skipping `handle_uncommitted_changes` (because no changes).
    fn mock_no_changes_but_ahead() -> MockCommandRunner {
        let mut runner = MockCommandRunner::new();
        let empty_success =
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() };
        let ahead_of_remote =
            CommandOutput { exit_code: 0, stdout: "1\n".to_string(), stderr: String::new() };

        // First check_uncommitted_changes (fast path check at line 281)
        runner.expect("git", &["diff", "--stat"], empty_success.clone());
        runner.expect("git", &["diff", "--cached", "--stat"], empty_success.clone());
        runner.expect(
            "git",
            &["ls-files", "--others", "--exclude-standard"],
            empty_success.clone(),
        );
        runner.expect(
            "git",
            &["rev-list", "--count", "@{upstream}..HEAD"],
            ahead_of_remote.clone(),
        );

        // Second check_uncommitted_changes (at line 420)
        runner.expect("git", &["diff", "--stat"], empty_success.clone());
        runner.expect("git", &["diff", "--cached", "--stat"], empty_success.clone());
        runner.expect("git", &["ls-files", "--others", "--exclude-standard"], empty_success);
        runner.expect("git", &["rev-list", "--count", "@{upstream}..HEAD"], ahead_of_remote);

        runner
    }

    #[test]
    fn test_run_stop_hook_blocks_after_reflection_with_incomplete_requested_tasks() {
        use crate::tasks::{Priority, SqliteTaskStore, TaskStore};

        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // Set up database directory
        let db_path = crate::paths::project_db_path(base);
        std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();

        // Set the reflection marker (simulating a second stop after work)
        session::set_reflect_marker(base).unwrap();

        // Create and request a task
        let store = SqliteTaskStore::for_project(base).unwrap();
        let task = store.create_task("Incomplete task", "", Priority::High).unwrap();
        store.request_tasks(&[&task.id]).unwrap();

        // Mock git: no changes but ahead of remote (skips fast path AND handle_uncommitted_changes)
        // require_push is false by default, so we won't block on unpushed commits
        let runner = mock_no_changes_but_ahead();
        let sub_agent = MockSubAgent::new();

        let input = crate::hooks::HookInput::default();
        let config = StopHookConfig {
            git_repo: true,
            base_dir: Some(base.to_path_buf()),
            ..Default::default()
        };

        let result = run_stop_hook(&input, &config, &runner, &sub_agent).unwrap();

        // Should block because there's an incomplete requested task
        assert!(
            !result.allow_stop,
            "Expected stop to be blocked but got allow. Messages: {:?}",
            result.messages
        );
        assert!(
            result.messages.iter().any(|m| m.contains("Requested Tasks Incomplete")),
            "Expected 'Requested Tasks Incomplete' message but got: {:?}",
            result.messages
        );

        // Reflection marker should be cleared because we went through the reflection path
        assert!(!session::has_reflect_marker(base));
    }

    #[test]
    fn test_run_stop_hook_blocks_when_asking_question_with_modifying_tools_and_requested_tasks() {
        use crate::tasks::{Priority, SqliteTaskStore, TaskStore};

        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // Set up database directory
        let db_path = crate::paths::project_db_path(base);
        std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();

        // Create and request a task
        let store = SqliteTaskStore::for_project(base).unwrap();
        let task = store.create_task("Incomplete task", "", Priority::High).unwrap();
        store.request_tasks(&[&task.id]).unwrap();

        // Set up git to have uncommitted changes
        let runner = mock_uncommitted_changes();
        let sub_agent = MockSubAgent::new();

        // Transcript: agent has done modifying work and is now asking a question
        let transcript_content = r#"{"type": "user", "timestamp": "2024-01-01T12:00:00Z", "message": {"role": "user", "content": "Do the task"}}
{"type": "assistant", "message": {"content": [{"type": "tool_use", "name": "Edit", "id": "1"}]}}
{"type": "assistant", "message": {"content": [{"type": "text", "text": "I made some changes. What should I do next?"}]}}
"#;
        let transcript_file = base.join("transcript.jsonl");
        std::fs::write(&transcript_file, transcript_content).unwrap();

        let input = crate::hooks::HookInput {
            transcript_path: Some(transcript_file.to_string_lossy().to_string()),
            ..Default::default()
        };
        let config = StopHookConfig {
            git_repo: true,
            base_dir: Some(base.to_path_buf()),
            ..Default::default()
        };

        let result = run_stop_hook(&input, &config, &runner, &sub_agent).unwrap();

        // Should block because there's an incomplete requested task and modifying work was done
        assert!(!result.allow_stop);
        assert!(result.messages.iter().any(|m| m.contains("Requested Tasks Incomplete")));
    }

    #[test]
    fn test_run_stop_hook_blocks_on_question_path_with_requested_tasks() {
        // This test specifically covers line 492: the question check path (line 487-494)
        // which is AFTER the reflection marker check, reached when:
        // 1. git_repo = false (skip all git checks)
        // 2. No reflection marker set
        // 3. Agent output looks like a question
        // 4. Has modifying tool use
        // 5. Has incomplete requested tasks
        use crate::tasks::{Priority, SqliteTaskStore, TaskStore};

        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // Set up database directory
        let db_path = crate::paths::project_db_path(base);
        std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();

        // Create and request a task
        let store = SqliteTaskStore::for_project(base).unwrap();
        let task = store.create_task("Incomplete task", "", Priority::High).unwrap();
        store.request_tasks(&[&task.id]).unwrap();

        // No git repo - skip all git checks
        let runner = MockCommandRunner::new();
        let sub_agent = MockSubAgent::new();

        // Transcript: agent has done modifying work and is asking a question
        let transcript_content = r#"{"type": "user", "timestamp": "2024-01-01T12:00:00Z", "message": {"role": "user", "content": "Do the task"}}
{"type": "assistant", "message": {"content": [{"type": "tool_use", "name": "Edit", "id": "1"}]}}
{"type": "assistant", "message": {"content": [{"type": "text", "text": "What should I do next?"}]}}
"#;
        let transcript_file = base.join("transcript.jsonl");
        std::fs::write(&transcript_file, transcript_content).unwrap();

        let input = crate::hooks::HookInput {
            transcript_path: Some(transcript_file.to_string_lossy().to_string()),
            ..Default::default()
        };
        // git_repo = false to skip all git-related checks
        let config = StopHookConfig {
            git_repo: false,
            base_dir: Some(base.to_path_buf()),
            ..Default::default()
        };

        let result = run_stop_hook(&input, &config, &runner, &sub_agent).unwrap();

        // Should block because there's an incomplete requested task
        assert!(
            !result.allow_stop,
            "Expected stop to be blocked but got allow. Messages: {:?}",
            result.messages
        );
        assert!(
            result.messages.iter().any(|m| m.contains("Requested Tasks Incomplete")),
            "Expected 'Requested Tasks Incomplete' message but got: {:?}",
            result.messages
        );
    }

    #[test]
    fn test_check_validation_required_no_command() {
        // Test that check_validation_required returns None when needs_validation is true
        // but no quality_check_command is configured
        let dir = TempDir::new().unwrap();
        let config = StopHookConfig {
            base_dir: Some(dir.path().to_path_buf()),
            quality_check_command: None, // No command configured
            ..Default::default()
        };

        // Set the validation marker
        session::set_needs_validation(dir.path()).unwrap();

        let runner = MockCommandRunner::new();
        let result = check_validation_required(&config, &runner).unwrap();
        assert!(result.is_none(), "Expected None when no quality_check_command configured");
    }

    #[test]
    fn test_check_quality_gate_enabled_no_command() {
        // Test that check_quality_gate returns None when quality_check_enabled is true
        // but no quality_check_command is configured
        let dir = TempDir::new().unwrap();
        let config = StopHookConfig {
            base_dir: Some(dir.path().to_path_buf()),
            quality_check_enabled: true,
            quality_check_command: None, // No command configured
            ..Default::default()
        };

        let runner = MockCommandRunner::new();
        let result = check_quality_gate(&config, &runner).unwrap();
        assert!(result.is_none(), "Expected None when no quality_check_command configured");
    }

    #[test]
    fn test_check_quality_gate_passes() {
        // Test that check_quality_gate returns None when quality check passes
        let dir = TempDir::new().unwrap();
        let config = StopHookConfig {
            base_dir: Some(dir.path().to_path_buf()),
            quality_check_enabled: true,
            quality_check_command: Some("echo 'ok'".to_string()),
            ..Default::default()
        };

        let mut runner = MockCommandRunner::new();
        runner.expect(
            "sh",
            &["-c", "echo 'ok'"],
            CommandOutput { exit_code: 0, stdout: "ok\n".to_string(), stderr: String::new() },
        );

        let result = check_quality_gate(&config, &runner).unwrap();
        assert!(result.is_none(), "Expected None when quality check passes");
        runner.verify();
    }
}
