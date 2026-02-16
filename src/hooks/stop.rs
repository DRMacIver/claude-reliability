//! Stop hook for code quality checks.
//!
//! This hook runs when Claude attempts to stop/exit. It implements:
//! - Uncommitted changes detection and blocking
//! - Quality checks via configured command
//! - Interactive question handling with sub-agent
//! - Task completion tracking

use crate::error::{Error, Result};
use crate::git::{self, GitStatus};
use crate::hooks::HookInput;
use crate::question::{is_continue_question, looks_like_question, truncate_for_context};
use crate::session;
use crate::tasks;
use crate::templates;
use crate::traits::{CommandRunner, QuestionContext, SubAgent, SubAgentDecision};
use crate::transcript::{self, is_simple_question, TranscriptInfo};
use chrono::{DateTime, Utc};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tera::Context;

/// Time window for considering user as "recently active" (minutes).
pub const USER_RECENCY_MINUTES: u32 = 5;

/// Timeout for quality check commands (5 minutes).
pub const QUALITY_CHECK_TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// Accumulator for tracking which checks have been run and their results.
#[derive(Debug, Default)]
struct ChecksLog {
    entries: Vec<String>,
}

impl ChecksLog {
    /// Log a check that passed (returned None, continuing to next check).
    fn pass(&mut self, check_name: &str, detail: &str) {
        self.entries.push(format!("  {check_name}: {detail}"));
    }

    /// Create a result with the accumulated log entries and add formatted log to messages.
    fn into_result(self, mut result: StopHookResult) -> StopHookResult {
        result.checks_log = self.entries;
        // Add formatted checks log to messages for display
        let formatted = result.format_checks_log();
        if !formatted.is_empty() {
            result.messages.insert(0, formatted);
        }
        result
    }
}

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
    /// When set, constrains the session to a single assigned work item.
    /// Read from `CLAUDE_RELIABILITY_SINGLE_WORK_ITEM` at config construction time.
    pub single_work_item_id: Option<String>,
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
    /// Log of all checks that were run and their results.
    pub checks_log: Vec<String>,
}

impl StopHookResult {
    /// Create an "allow" result.
    pub const fn allow() -> Self {
        Self {
            allow_stop: true,
            exit_code: 0,
            messages: Vec::new(),
            inject_response: None,
            checks_log: Vec::new(),
        }
    }

    /// Create a "block" result.
    pub const fn block() -> Self {
        Self {
            allow_stop: false,
            exit_code: 2,
            messages: Vec::new(),
            inject_response: None,
            checks_log: Vec::new(),
        }
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

    /// Format the checks log as a displayable string.
    fn format_checks_log(&self) -> String {
        if self.checks_log.is_empty() {
            return String::new();
        }
        let header = if self.allow_stop { "Stop allowed:" } else { "Stop blocked:" };
        format!("{}\n{}", header, self.checks_log.join("\n"))
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
///
/// # Panics
///
/// Panics if exiting problem mode fails (database error).
fn check_problem_mode_exit(config: &StopHookConfig) -> Option<StopHookResult> {
    if session::is_problem_mode_active(config.base_dir()) {
        session::exit_problem_mode(config.base_dir()).expect("failed to exit problem mode");
        let message = templates::render("messages/stop/problem_mode_exit.tera", &Context::new())
            .expect("problem_mode_exit.tera template should always render");
        return Some(
            StopHookResult::allow()
                .with_message(message)
                .with_explanation(config.explain_stops, "problem mode was active"),
        );
    }
    None
}

/// Default base backoff time for overloaded errors (5 seconds).
const DEFAULT_BACKOFF_BASE_SECS: u64 = 5;

/// Default maximum backoff time for overloaded errors (2 minutes).
const DEFAULT_BACKOFF_MAX_SECS: u64 = 120;

/// Configuration for overloaded error backoff.
#[derive(Debug, Clone, Copy)]
pub struct BackoffConfig {
    /// Base backoff time in seconds.
    pub base_secs: u64,
    /// Maximum total wait time in seconds before allowing stop.
    pub max_secs: u64,
}

impl Default for BackoffConfig {
    fn default() -> Self {
        Self { base_secs: DEFAULT_BACKOFF_BASE_SECS, max_secs: DEFAULT_BACKOFF_MAX_SECS }
    }
}

/// Result of calculating backoff for overloaded errors.
#[derive(Debug, PartialEq, Eq)]
pub enum OverloadedBackoffResult {
    /// Should retry after sleeping for the specified duration.
    Retry { sleep_secs: u64, error_count: u32 },
    /// Should allow stop - exceeded maximum retry time.
    AllowStop { error_count: u32, total_wait: u64 },
}

/// Calculate backoff for overloaded errors with default configuration.
///
/// Returns `Retry` with sleep duration if retries should continue,
/// or `AllowStop` if maximum retry time has been exceeded.
#[cfg(test)]
pub fn calculate_overloaded_backoff(error_count: u32) -> OverloadedBackoffResult {
    calculate_overloaded_backoff_with_config(error_count, &BackoffConfig::default())
}

/// Calculate backoff for overloaded errors with custom configuration.
///
/// Returns `Retry` with sleep duration if retries should continue,
/// or `AllowStop` if maximum retry time has been exceeded.
pub fn calculate_overloaded_backoff_with_config(
    error_count: u32,
    config: &BackoffConfig,
) -> OverloadedBackoffResult {
    // Calculate backoff: base * 2^(error_count - 1), capped at max
    let backoff_secs = std::cmp::min(
        config.base_secs.saturating_mul(1 << (error_count.saturating_sub(1))),
        config.max_secs,
    );

    // Calculate total wait time so far: sum of all backoffs
    // Sum of base * (1 + 2 + 4 + ... + 2^(n-1)) = base * (2^n - 1)
    let capped_count = std::cmp::min(error_count, 6); // After 6 errors we're at max
    let total_wait = config.base_secs.saturating_mul((1 << capped_count) - 1);

    // If we've waited more than max total, allow stop
    if total_wait > config.max_secs {
        return OverloadedBackoffResult::AllowStop { error_count, total_wait };
    }

    // Add jitter (±20% of backoff time)
    let jitter_range = backoff_secs / 5; // 20%
    let sleep_secs = if jitter_range > 0 {
        use std::time::{SystemTime, UNIX_EPOCH};
        let seed = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        // Simple pseudo-random: take low bits of nanoseconds
        // jitter_offset is in [0, 2 * jitter_range]
        #[allow(clippy::cast_possible_truncation)] // Jitter is small, won't overflow u64
        let jitter_offset = (seed % u128::from(jitter_range * 2 + 1)) as u64;

        // Apply jitter: if offset > range, add (positive jitter); else subtract (negative jitter)
        if jitter_offset > jitter_range {
            backoff_secs.saturating_add(jitter_offset - jitter_range)
        } else {
            backoff_secs.saturating_sub(jitter_range - jitter_offset)
        }
    } else {
        backoff_secs
    };

    OverloadedBackoffResult::Retry { sleep_secs, error_count }
}

/// Check for API error loop and allow exit to prevent infinite loops.
///
/// If we've seen multiple consecutive API errors, allow the stop to prevent
/// the agent from getting stuck in an error loop.
///
/// For 529 overloaded errors specifically, implements exponential backoff
/// with retry instead of immediately allowing stop.
fn check_api_error_loop(
    transcript_info: &TranscriptInfo,
    config: &StopHookConfig,
) -> Option<StopHookResult> {
    check_api_error_loop_with_backoff(transcript_info, config, &BackoffConfig::default())
}

/// Check for API error loop with configurable backoff (for testing).
fn check_api_error_loop_with_backoff(
    transcript_info: &TranscriptInfo,
    config: &StopHookConfig,
    backoff_config: &BackoffConfig,
) -> Option<StopHookResult> {
    // Handle 529 overloaded errors with exponential backoff
    if transcript_info.last_error_is_overloaded && transcript_info.consecutive_api_errors > 0 {
        return Some(handle_overloaded_error_with_backoff(transcript_info, config, backoff_config));
    }

    // For other API errors, use the threshold-based approach
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

/// Handle 529 overloaded errors with configurable backoff.
fn handle_overloaded_error_with_backoff(
    transcript_info: &TranscriptInfo,
    config: &StopHookConfig,
    backoff_config: &BackoffConfig,
) -> StopHookResult {
    let error_count = transcript_info.consecutive_api_errors;

    match calculate_overloaded_backoff_with_config(error_count, backoff_config) {
        OverloadedBackoffResult::AllowStop { error_count, total_wait } => {
            StopHookResult::allow()
                .with_message(format!(
                    "# API Overloaded - Maximum Retry Time Exceeded\n\n\
                     The API has been overloaded for too long ({error_count} consecutive errors, ~{total_wait}s total wait time).\n\n\
                     Please start a new conversation to continue."
                ))
                .with_explanation(
                    config.explain_stops,
                    format!("{error_count} overloaded errors, exceeded maximum retry time"),
                )
        }
        OverloadedBackoffResult::Retry { sleep_secs, error_count } => {
            // Sleep for the backoff period
            eprintln!(
                "API overloaded (error {error_count}/~6). Waiting {sleep_secs}s before retry..."
            );
            std::thread::sleep(std::time::Duration::from_secs(sleep_secs));

            // Return a block result asking the agent to retry
            StopHookResult::block()
                .with_inject(
                    "The API was temporarily overloaded. Please try your last request again."
                        .to_string(),
                )
                .with_explanation(
                    config.explain_stops,
                    format!("overloaded error {error_count}, waited {sleep_secs}s, prompting retry"),
                )
        }
    }
}

/// Check if a message contains commit or push keywords.
///
/// Case-insensitive check for "commit" or "push" in the message text.
fn contains_commit_push_keywords(msg: &str) -> bool {
    let lower = msg.to_ascii_lowercase();
    lower.contains("commit") || lower.contains("push")
}

/// Check for simple Q&A exchange that should allow immediate exit.
///
/// If the last user message is a simple question and no modifications were made
/// since then, this is a clarifying Q&A - allow immediate stop (skip reflection).
/// The agent can answer however it wants (long answers, follow-up questions, etc.)
///
/// Guard clauses prevent the fast path when:
/// - The user message contains commit/push keywords (fall through to auto-confirm)
/// - The git repo has uncommitted changes or unpushed commits (work needs landing)
///
/// # Errors
///
/// Returns an error if git commands fail.
fn check_simple_qa_fast_path(
    transcript_info: &TranscriptInfo,
    config: &StopHookConfig,
    runner: &dyn CommandRunner,
) -> Result<Option<StopHookResult>> {
    if !transcript_info.has_modifying_tool_use_since_user {
        if let Some(ref last_user_msg) = transcript_info.last_user_message {
            if is_simple_question(last_user_msg) {
                // Don't swallow messages like "Can you commit this?"
                if contains_commit_push_keywords(last_user_msg) {
                    return Ok(None);
                }

                // Don't allow fast path when git repo has dirty state
                if config.git_repo {
                    let git_status = git::check_uncommitted_changes(runner)?;
                    if git_status.uncommitted.has_changes() {
                        return Ok(None);
                    }
                    if config.require_push && git_status.ahead_of_remote {
                        return Ok(None);
                    }
                }

                return Ok(Some(StopHookResult::allow().with_explanation(
                    config.explain_stops,
                    "simple Q&A with no modifications since question",
                )));
            }
        }
    }
    Ok(None)
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
///
/// # Errors
///
/// Returns an error if running the validation command fails.
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

    // Run the validation command with timeout
    let output = match runner.run("sh", &["-c", check_cmd], Some(QUALITY_CHECK_TIMEOUT)) {
        Ok(output) => output,
        Err(Error::CommandTimeout { command, timeout_secs }) => {
            // Timeout - block exit with special message
            let result = StopHookResult::block()
                .with_message("# Quality Check Timeout")
                .with_message("")
                .with_message(format!(
                    "The quality check command `{command}` took longer than {} minutes and was stopped.",
                    timeout_secs / 60
                ))
                .with_message("")
                .with_message("This indicates a problem that needs to be fixed:")
                .with_message("- The check command may be hanging or stuck")
                .with_message("- There may be an infinite loop or deadlock")
                .with_message("- The test suite may have a performance regression")
                .with_message("")
                .with_message("Please investigate and fix the issue before continuing.");
            return Ok(Some(result));
        }
        Err(e) => return Err(e),
    };

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
    session::clear_needs_validation(config.base_dir()).expect("failed to clear validation marker");
    Ok(None)
}

// =============================================================================
// Tier 3: Bypass Phrase Handling
// =============================================================================

/// Check if an emergency stop has been requested via the `emergency-stop` CLI command.
///
/// If the emergency stop marker is active, clears it and allows stopping.
fn check_emergency_stop_exit(config: &StopHookConfig) -> Option<StopHookResult> {
    if !session::is_emergency_stop_active(config.base_dir()) {
        return None;
    }

    // Clear the marker and allow stop
    let _ = session::clear_emergency_stop(config.base_dir());
    Some(StopHookResult::allow().with_message("Emergency stop activated. Allowing stop."))
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

    // In single work item mode, check only the assigned item
    if let Some(single_id) = &config.single_work_item_id {
        return check_single_work_item_complete(config, single_id);
    }

    check_incomplete_requested_tasks(config)
}

// =============================================================================
// Tier 4: Git State Checks
// =============================================================================

/// Check for uncommitted changes and block if present.
///
/// # Errors
///
/// Returns an error if git commands fail or if handling uncommitted changes fails.
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
) -> Option<StopHookResult> {
    check_interactive_question(transcript_info, sub_agent, config)
}

// =============================================================================
// Tier 5b: Work Item Reminder
// =============================================================================

/// Check if the agent should be reminded to create follow-up work items.
///
/// On the first stop with modifying tool use, prompt the agent to consider
/// creating work items. The marker is set once and never cleared by the stop
/// hook, so subsequent stops pass through.
///
/// # Panics
///
/// Panics if setting the work item reminded marker fails (database error).
fn check_work_item_reminder(
    transcript_info: &TranscriptInfo,
    config: &StopHookConfig,
) -> Option<StopHookResult> {
    if !transcript_info.has_modifying_tool_use {
        return None;
    }

    let base_dir = config.base_dir();
    if session::has_work_item_reminded(base_dir) {
        return None;
    }

    // First stop with modifying tools - set marker and prompt
    session::set_work_item_reminded(base_dir).expect("failed to set work item reminded marker");
    let message = templates::render("messages/stop/work_item_reminder.tera", &tera::Context::new())
        .expect("work_item_reminder.tera template should always render");
    Some(StopHookResult::block().with_message(message))
}

// =============================================================================
// Tier 6: Reflection Checks
// =============================================================================

/// Create work items in the task store for incomplete reflection items.
///
/// Returns a list of formatted strings like `- [ID] Title` for each successfully created item.
/// Returns an empty vec if the task store can't be opened or task creation fails.
fn create_reflection_work_items(base_dir: &Path, items: &[String]) -> Vec<String> {
    use crate::tasks::{Priority, SqliteTaskStore, TaskStore};

    let mut created = Vec::new();
    if let Ok(store) = SqliteTaskStore::for_project(base_dir) {
        for title in items {
            if let Ok(task) =
                store.create_task(title, "Created from reflection evaluation", Priority::Medium)
            {
                created.push(format!("- [{}] {}", task.id, title));
            }
        }
    }
    created
}

/// Format the work list for display.
///
/// Uses created task titles (with IDs) if available, otherwise falls back to plain items.
fn format_work_list(items: &[String], created_titles: &[String]) -> String {
    if created_titles.is_empty() {
        items.iter().map(|t| format!("- {t}")).collect::<Vec<_>>().join("\n")
    } else {
        created_titles.join("\n")
    }
}

/// Check if reflection was already prompted and allow stop.
///
/// If the agent got the reflection prompt and is stopping again, allow it.
/// Also checks for incomplete requested tasks before allowing.
///
/// # Panics
///
/// Panics if clearing the reflect marker fails (database error).
fn check_reflection_marker_allow(
    config: &StopHookConfig,
    session_id: &str,
    transcript_info: &TranscriptInfo,
    sub_agent: &dyn SubAgent,
) -> Option<StopHookResult> {
    use crate::traits::ReflectionContext;

    let base_dir = config.base_dir();
    if !session::has_reflect_marker(base_dir) {
        return None;
    }

    // Agent already got the reflection prompt and is stopping again - allow it
    session::clear_reflect_marker(base_dir).expect("failed to clear reflect marker");

    // Evaluate the reflection content via sub-agent
    if let Some(ref reflection_output) = transcript_info.last_assistant_output {
        let user_messages: Vec<String> = tasks::get_session_user_messages(base_dir, session_id)
            .into_iter()
            .map(|m| m.message)
            .collect();

        let context =
            ReflectionContext { reflection_output: reflection_output.clone(), user_messages };

        match sub_agent.evaluate_reflection(&context) {
            Ok(crate::traits::ReflectionDecision::Incomplete { items }) => {
                // Create work items for each incomplete item
                let created_titles = create_reflection_work_items(base_dir, &items);

                // Re-set reflect marker so next stop re-evaluates
                session::set_reflect_marker(base_dir).expect("failed to re-set reflect marker");

                let work_list = format_work_list(&items, &created_titles);

                return Some(StopHookResult::block().with_message(format!(
                    "## Incomplete Work Detected\n\n\
                         Your reflection indicates the following work items remain:\n\n\
                         {work_list}\n\n\
                         Work items have been created. Please complete them before stopping."
                )));
            }
            Ok(crate::traits::ReflectionDecision::Complete) | Err(_) => {
                // Complete or sub-agent error — fall through to existing checks
            }
        }
    }

    // Check for incomplete requested tasks before allowing stop (work was done)
    // In single work item mode, check only the assigned item
    if let Some(single_id) = &config.single_work_item_id {
        if let Some(result) = check_single_work_item_complete(config, single_id) {
            return Some(result);
        }
    } else if let Some(result) = check_incomplete_requested_tasks(config) {
        return Some(result);
    }

    // Note: check_auto_work_tasks already ran before this in run_stop_hook,
    // so we don't need to check it again here.

    // Clear user messages for this session since reflection is complete
    tasks::clear_session_user_messages(base_dir, session_id);

    Some(
        StopHookResult::allow()
            .with_explanation(config.explain_stops, "reflection already prompted on first stop"),
    )
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

/// Truncate a message for display in the reflection prompt.
fn truncate_message(message: &str, max_len: usize) -> String {
    let collapsed: String = message.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.len() <= max_len {
        return collapsed;
    }
    let mut end = max_len;
    while end > 0 && !collapsed.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", &collapsed[..end])
}

/// Prompt for reflection on first stop if modifying tools were used.
///
/// Includes user messages from the session in the reflection prompt
/// so the agent can verify all requests were addressed.
///
/// # Panics
///
/// Panics if setting the reflect marker fails (database error).
fn check_reflection_prompt(
    transcript_info: &TranscriptInfo,
    config: &StopHookConfig,
    session_id: &str,
) -> Option<StopHookResult> {
    if !transcript_info.has_modifying_tool_use {
        return None;
    }

    // Modifying tools were used, prompt for reflection
    session::set_reflect_marker(config.base_dir()).expect("failed to set reflect marker");

    // Get user messages for this session
    let messages = tasks::get_session_user_messages(config.base_dir(), session_id);

    let mut result =
        StopHookResult::block().with_message("# Task Completion Check").with_message("");

    if messages.is_empty() {
        result = result
            .with_message(
                "Before exiting, carefully analyze whether you have fully completed the task.",
            )
            .with_message("")
            .with_message("If you have NOT completed the task:")
            .with_message("  - Continue working to finish it")
            .with_message("")
            .with_message("If you HAVE completed the task:")
            .with_message("  - Provide a clear, concise summary of what was done for the user")
            .with_message("  - Then stop again to exit");
    } else {
        result = result.with_message(
            "Before exiting, verify that ALL user messages in this session have been fully addressed:",
        ).with_message("");

        for (i, msg) in messages.iter().enumerate() {
            let pre_compaction_note = if msg.pre_compaction {
                " (from before context compaction - MUST still be verified)"
            } else {
                ""
            };
            let truncated = truncate_message(&msg.message, 200);
            result = result.with_message(format!(
                "{}. [{}] ({}{}): \"{}\"",
                i + 1,
                msg.created_at,
                msg.context,
                pre_compaction_note,
                truncated
            ));
        }

        result = result
            .with_message("")
            .with_message("For each message above, confirm it has been fully addressed.")
            .with_message("")
            .with_message("If any requests are NOT complete:")
            .with_message("  - Continue working to finish them")
            .with_message("")
            .with_message("If ALL requests HAVE been addressed:")
            .with_message("  - Provide a clear, concise summary of what was done")
            .with_message("  - Then stop again to exit");
    }

    Some(result)
}

/// Check if we should prompt to work on open tasks (final check before allow).
/// Returns (result, reason) where reason explains the outcome.
fn check_auto_work_tasks_block(
    config: &StopHookConfig,
    transcript_info: &TranscriptInfo,
) -> (Option<StopHookResult>, &'static str) {
    check_auto_work_tasks(config, transcript_info)
}

/// Run the stop hook.
///
/// This function orchestrates all stop checks in a specific order. Each check
/// can either allow the stop, block it, or pass through to the next check.
///
/// # Check Order
///
/// 1. **Tier 1 - Fast exits**: Problem mode, emergency stop, API errors, simple Q&A, auto-confirm commit/push
/// 2. **Tier 2 - Validation**: Run quality checks if configured
/// 3. **Tier 3 - Task completion**: Requested tasks, auto-work tasks
/// 4. **Tier 4 - Git state**: Uncommitted changes block
/// 5. **Tier 5 - Work item reminder**: Prompt to create follow-up work items (first stop only)
/// 6. **Tier 6 - Reflection**: Reflection markers, question skip, reflection prompts
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
    // FIXME: The return types of the functions this is broken up into
    // need some work. Option<Result> and Result<Option> are both to be
    // avoided where possible. Define some custom enums? Or just stop using
    // Result. I think every error path through here can just be allowed to
    // panic.

    // Parse transcript if available
    let transcript_info = input
        .transcript_path
        .as_ref()
        .and_then(|p| transcript::parse_transcript(Path::new(p)).ok())
        .unwrap_or_default();

    // Track all checks that are run
    let mut log = ChecksLog::default();

    // =========================================================================
    // Tier 1: Fast Exit Checks
    // =========================================================================

    // Always stop immediately in API error loops
    if let Some(r) = check_api_error_loop(&transcript_info, config) {
        log.pass("api_error_loop", "detected, allowing stop");
        return Ok(log.into_result(r));
    }
    log.pass("api_error_loop", "no errors");

    // The agent has previously said it has run into an insurmountable problem
    // and was asked to explain it. Now it has.
    if let Some(r) = check_problem_mode_exit(config) {
        log.pass("problem_mode_exit", "in problem mode, allowing stop");
        return Ok(log.into_result(r));
    }
    log.pass("problem_mode_exit", "not in problem mode");

    // The agent has used the emergency_stop tool and it was accepted.
    if let Some(r) = check_emergency_stop_exit(config) {
        log.pass("emergency_stop_exit", "emergency stop active, allowing stop");
        return Ok(log.into_result(r));
    }
    log.pass("emergency_stop_exit", "no emergency stop");

    // The agent has not yet done any work, and has asked a clarifying question,
    // which should be allowed automatically.
    if let Some(r) = check_simple_qa_fast_path(&transcript_info, config, runner)? {
        log.pass("simple_qa_fast_path", "simple Q&A, allowing stop");
        return Ok(log.into_result(r));
    }
    log.pass("simple_qa_fast_path", "not simple Q&A");

    // If the agent has asked if it should commit or push, auto-confirm.
    // This must run BEFORE interactive_question check to prevent commit/push
    // questions from being treated as interactive questions that allow stopping.
    if let Some(r) = check_commit_push_auto_confirm(&transcript_info, config) {
        log.pass("commit_push_auto_confirm", "auto-confirming commit/push");
        return Ok(log.into_result(r));
    }
    log.pass("commit_push_auto_confirm", "no commit/push question");

    // The agent has asked a question. Decide now whether to permit it.
    if let Some(r) = check_interactive_question_block(&transcript_info, sub_agent, config) {
        let action = if r.allow_stop { "allowing stop" } else { "blocking" };
        log.pass("interactive_question", action);
        return Ok(log.into_result(r));
    }
    log.pass("interactive_question", "no interactive question");

    // =========================================================================
    // Tier 2: Validation Checks
    // =========================================================================

    // If any changes have been made, the validation step needs to be run.
    if let Some(r) = check_validation_required(config, runner)? {
        log.pass("validation_required", "validation failed, blocking");
        return Ok(log.into_result(r));
    }
    log.pass("validation_required", "passed or not needed");

    // =========================================================================
    // Tier 3: Task Completion Checks
    // =========================================================================

    // There are outstanding requested tasks, so the agent is not allowed to stop.
    if let Some(r) = check_requested_tasks_block(&transcript_info, config) {
        log.pass("requested_tasks", "incomplete requested tasks, blocking");
        return Ok(log.into_result(r));
    }
    log.pass("requested_tasks", "no incomplete requested tasks");

    // Prompt agent to work on open tasks if user has been idle.
    let (auto_work_result, auto_work_reason) =
        check_auto_work_tasks_block(config, &transcript_info);
    if let Some(r) = auto_work_result {
        log.pass("auto_work_tasks", auto_work_reason);
        return Ok(log.into_result(r));
    }
    log.pass("auto_work_tasks", auto_work_reason);

    // Cannot exit with uncommitted changes.
    if let Some(r) = check_uncommitted_changes_block(config, runner, &transcript_info, sub_agent)? {
        log.pass("uncommitted_changes", "uncommitted changes, blocking");
        return Ok(log.into_result(r));
    }
    log.pass("uncommitted_changes", "no uncommitted changes");

    // Remind agent to create follow-up work items (first stop only).
    if let Some(r) = check_work_item_reminder(&transcript_info, config) {
        log.pass("work_item_reminder", "prompting for work items");
        return Ok(log.into_result(r));
    }
    log.pass("work_item_reminder", "already reminded or no modifying tools");

    // Derive session ID for user message tracking.
    let session_id = input.transcript_path.as_deref().unwrap_or("unknown");

    // The model has previously been asked to reflect, and now it has.
    if let Some(r) = check_reflection_marker_allow(config, session_id, &transcript_info, sub_agent)
    {
        log.pass("reflection_marker", "reflection complete, allowing stop");
        return Ok(log.into_result(r));
    }
    log.pass("reflection_marker", "no reflection marker");

    // The model is asking a question - skip reflection.
    if let Some(r) = check_question_skip_reflection(&transcript_info, config) {
        log.pass("question_skip_reflection", "question asked, skipping reflection");
        return Ok(log.into_result(r));
    }
    log.pass("question_skip_reflection", "not a question");

    // Prompt for reflection before allowing stop.
    if let Some(r) = check_reflection_prompt(&transcript_info, config, session_id) {
        log.pass("reflection_prompt", "prompting for reflection");
        return Ok(log.into_result(r));
    }
    log.pass("reflection_prompt", "no reflection needed");

    // All checks passed - allow stop
    Ok(log.into_result(StopHookResult::allow()))
}

/// Check if we should prompt the agent to work on open tasks.
///
/// This is triggered when:
/// 1. `auto_work_on_tasks` is enabled
/// 2. There are open, ready tasks in the database
/// 3. User has been idle for at least `auto_work_idle_minutes`
///
/// Returns (Some(block result), reason) if agent should work on tasks,
/// (None, reason) otherwise explaining why auto-work didn't trigger.
fn check_auto_work_tasks(
    config: &StopHookConfig,
    transcript_info: &TranscriptInfo,
) -> (Option<StopHookResult>, &'static str) {
    // Skip if auto-work is disabled
    if !config.auto_work_on_tasks {
        return (None, "auto-work disabled");
    }

    // Check user idle time
    let user_idle_minutes = transcript_info.last_user_message_time.map_or(u32::MAX, |ts| {
        let now = Utc::now();
        let minutes = now.signed_duration_since(ts).num_minutes();
        // Clamp to u32 range (negative means user is in the future, treat as just now)
        u32::try_from(minutes.max(0)).unwrap_or(u32::MAX)
    }); // If no timestamp, treat as very idle

    if user_idle_minutes < config.auto_work_idle_minutes {
        return (None, "user active recently");
    }

    // Check for open tasks
    let base_dir = config.base_dir();
    let ready_task_count = tasks::count_ready_tasks(base_dir);

    if ready_task_count == 0 {
        return (None, "no ready tasks");
    }

    // User is idle and there are tasks - prompt to work on them
    let mut ctx = Context::new();
    ctx.insert("task_count", &ready_task_count);
    ctx.insert("idle_minutes", &user_idle_minutes);

    let message = templates::render("messages/stop/auto_work_tasks.tera", &ctx)
        .expect("auto_work_tasks.tera template should always render");

    (Some(StopHookResult::block().with_message(message)), "prompting to work on tasks")
}

/// Check if there are incomplete requested tasks that block stopping.
///
/// Returns Some(block result) if there are incomplete requested tasks, None otherwise.
/// A task is considered "incomplete" if it's requested and not complete/abandoned,
/// unless it's blocked only on an unanswered question.
fn check_incomplete_requested_tasks(config: &StopHookConfig) -> Option<StopHookResult> {
    let incomplete = tasks::get_incomplete_requested_work(config.base_dir());

    if incomplete.is_empty() {
        // No incomplete requested tasks - also clear request mode since all are done
        tasks::clear_request_mode(config.base_dir());
        return None;
    }

    let mut result = StopHookResult::block()
        .with_message("# Requested Work Items Incomplete")
        .with_message("")
        .with_message(
            "The following work items were requested by the user and must be completed before stopping:",
        )
        .with_message("");

    for (id, title, status) in &incomplete {
        result = result.with_message(format!("- [{status}] {id}: {title}"));
    }

    result = result
        .with_message("")
        .with_message("Use `claude-reliability work update <id> --status complete` to mark items done. Run `/claude-reliability:task-management` for detailed guidance.")
        .with_message("")
        .with_message("If you've hit a problem you cannot solve without user input, use `claude-reliability emergency-stop <explanation>`.");

    Some(result)
}

/// Check if the single assigned work item is complete.
///
/// Returns `None` if the item is complete (allow stop).
/// Returns `Some(block)` if the item is still incomplete.
fn check_single_work_item_complete(
    config: &StopHookConfig,
    single_id: &str,
) -> Option<StopHookResult> {
    if crate::single_work_item::is_work_item_complete(config.base_dir(), single_id) {
        return None;
    }

    let result = StopHookResult::block()
        .with_message("# Single Work Item Incomplete")
        .with_message("")
        .with_message(format!("The assigned work item ({single_id}) has not been completed yet."))
        .with_message("")
        .with_message(
            "Use `claude-reliability work update <id> --status complete` to mark it done.",
        )
        .with_message("")
        .with_message(
            "If you've hit a problem you cannot solve without user input, \
             use `claude-reliability emergency-stop <explanation>`.",
        );

    Some(result)
}

/// Check if the assistant's last message is asking about committing or pushing.
/// Returns Some(response) if we should auto-confirm, None otherwise.
fn check_commit_push_question(output: &str) -> Option<String> {
    // Get the last sentence/question from the output
    let trimmed = output.trim();

    // Check for commit confirmation questions
    if trimmed.ends_with("Would you like me to commit these changes?")
        || trimmed.ends_with("Would you like me to commit these changes now?")
        || trimmed.ends_with("Would you like me to commit this?")
        || trimmed.ends_with("Would you like me to commit?")
        || trimmed.ends_with("Would you like me to commit now?")
        || trimmed.ends_with("Shall I commit these changes?")
        || trimmed.ends_with("Shall I commit these changes now?")
        || trimmed.ends_with("Shall I commit now?")
        || trimmed.ends_with("Should I commit these changes?")
        || trimmed.ends_with("Should I commit these changes now?")
        || trimmed.ends_with("Should I commit now?")
        || trimmed.ends_with("Ready to commit?")
    {
        return Some("Yes, please commit these changes.".to_string());
    }

    // Check for push confirmation questions
    if trimmed.ends_with("Would you like me to push these changes?")
        || trimmed.ends_with("Would you like me to push these changes now?")
        || trimmed.ends_with("Would you like me to push this?")
        || trimmed.ends_with("Would you like me to push?")
        || trimmed.ends_with("Would you like me to push now?")
        || trimmed.ends_with("Shall I push these changes?")
        || trimmed.ends_with("Shall I push these changes now?")
        || trimmed.ends_with("Shall I push now?")
        || trimmed.ends_with("Should I push these changes?")
        || trimmed.ends_with("Should I push these changes now?")
        || trimmed.ends_with("Should I push?")
        || trimmed.ends_with("Should I push now?")
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
    let mut quality_timed_out = false;
    if config.quality_check_enabled {
        if let Some(ref cmd) = config.quality_check_command {
            result.messages.push("# Running Quality Checks...".to_string());
            result.messages.push(String::new());
            match runner.run("sh", &["-c", cmd], Some(QUALITY_CHECK_TIMEOUT)) {
                Ok(output) => {
                    quality_passed = output.success();
                    quality_output = output.combined_output();
                }
                Err(Error::CommandTimeout { timeout_secs, .. }) => {
                    quality_passed = false;
                    quality_timed_out = true;
                    quality_output = format!(
                        "Quality check timed out after {} minutes. This indicates a problem that needs to be fixed.",
                        timeout_secs / 60
                    );
                }
                Err(e) => return Err(e),
            }
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
        if quality_timed_out {
            result.messages.push("## Quality Check Timeout".to_string());
            result.messages.push(String::new());
            result
                .messages
                .push("The quality check command took too long and was stopped.".to_string());
            result.messages.push(String::new());
            result.messages.push("This indicates a problem that needs to be fixed:".to_string());
            result.messages.push("- The check command may be hanging or stuck".to_string());
            result.messages.push("- There may be an infinite loop or deadlock".to_string());
            result.messages.push("- The test suite may have a performance regression".to_string());
        } else {
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
        .push("If you've hit a problem you cannot solve without user input, use the `emergency_stop` tool.".to_string());

    Ok(result)
}

/// Check for interactive question handling.
///
/// If the sub-agent fails (e.g. timeout), defaults to allowing the stop
/// so the hook doesn't panic.
fn check_interactive_question(
    transcript_info: &TranscriptInfo,
    sub_agent: &dyn SubAgent,
    config: &StopHookConfig,
) -> Option<StopHookResult> {
    let output = transcript_info.last_assistant_output.as_ref()?;

    // Check if it looks like a question
    if !looks_like_question(output) {
        return None;
    }

    // Check if user is recently active
    if !transcript::is_user_recently_active(transcript_info, USER_RECENCY_MINUTES) {
        return None;
    }

    // Truncate for context
    let truncated_output = truncate_for_context(output, 2000);

    // Fast path: Auto-answer "should I continue?" questions
    if is_continue_question(truncated_output) {
        return Some(
            StopHookResult::block()
                .with_message("# Fast path: Auto-answering continue question")
                .with_inject("Yes, please continue."),
        );
    }

    // Build question context for sub-agent
    let question_context = QuestionContext {
        assistant_output: truncated_output.to_string(),
        user_recency_minutes: USER_RECENCY_MINUTES,
        user_last_active: transcript_info.last_user_message_time.map(format_time_ago),
        has_modifications_since_user: transcript_info.has_modifying_tool_use_since_user,
    };

    // Run sub-agent decision. If the sub-agent fails (e.g. timeout), allow the stop
    // rather than panicking - a timeout likely means the agent is stuck anyway.
    let decision = match sub_agent.decide_on_question(&question_context) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Sub-agent failed to make decision: {e}");
            return Some(
                StopHookResult::allow()
                    .with_message("# Sub-agent Error")
                    .with_message("")
                    .with_message(format!("Sub-agent failed: {e}"))
                    .with_message("Allowing stop due to sub-agent failure."),
            );
        }
    };

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
            Some(result)
        }
        SubAgentDecision::Answer(answer) => Some(
            StopHookResult::block()
                .with_message("# Sub-agent Response")
                .with_message("")
                .with_message(&answer)
                .with_message("")
                .with_message("---")
                .with_message("Continuing work...")
                .with_inject(answer),
        ),
        SubAgentDecision::Continue => None,
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
            result.messages.iter().any(|m| m.contains("Requested Work Items Incomplete")),
            "Expected 'Requested Work Items Incomplete' message but got: {:?}",
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

        // Set markers since modifying tools trigger work item reminder then reflection
        session::set_work_item_reminded(base).unwrap();
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

        // Set markers since modifying tools trigger work item reminder then reflection
        session::set_work_item_reminded(base).unwrap();
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
    fn test_checks_log_included_in_messages() {
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
        // Should include the checks log
        let has_checks_log = result.messages.iter().any(|m| m.starts_with("Stop allowed:"));
        assert!(has_checks_log, "Expected checks log, got: {:?}", result.messages);
        // Should have individual check entries
        assert!(!result.checks_log.is_empty(), "Checks log should not be empty");
    }

    #[test]
    fn test_checks_log_always_included() {
        let dir = tempfile::TempDir::new().unwrap();
        let runner = mock_clean_git();
        let sub_agent = MockSubAgent::new();
        let input = crate::hooks::HookInput::default();
        let config = StopHookConfig {
            base_dir: Some(dir.path().to_path_buf()),
            explain_stops: false, // Even when false, checks log should be included
            ..Default::default()
        };

        let result = run_stop_hook(&input, &config, &runner, &sub_agent).unwrap();
        assert!(result.allow_stop);
        // Checks log should always be included, regardless of explain_stops
        let has_checks_log = result.messages.iter().any(|m| m.starts_with("Stop allowed:"));
        assert!(has_checks_log, "Checks log should always be included: {:?}", result.messages);
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
    fn test_format_checks_log_empty() {
        let result = StopHookResult::allow();
        assert!(result.format_checks_log().is_empty());
    }

    #[test]
    fn test_format_checks_log_with_entries() {
        let mut result = StopHookResult::allow();
        result.checks_log = vec!["  check1: passed".to_string(), "  check2: passed".to_string()];
        let formatted = result.format_checks_log();
        assert!(formatted.starts_with("Stop allowed:"));
        assert!(formatted.contains("check1: passed"));
        assert!(formatted.contains("check2: passed"));
    }

    #[test]
    fn test_format_checks_log_blocked() {
        let mut result = StopHookResult::block();
        result.checks_log = vec!["  check1: failed".to_string()];
        let formatted = result.format_checks_log();
        assert!(formatted.starts_with("Stop blocked:"));
    }

    #[test]
    fn test_check_auto_work_tasks_disabled() {
        let config = StopHookConfig { auto_work_on_tasks: false, ..Default::default() };
        let transcript = TranscriptInfo::default();

        let (result, reason) = check_auto_work_tasks(&config, &transcript);
        assert!(result.is_none());
        assert_eq!(reason, "auto-work disabled");
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

        let (result, reason) = check_auto_work_tasks(&config, &transcript);
        assert!(result.is_none()); // User is active, shouldn't block
        assert_eq!(reason, "user active recently");
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

        let (result, reason) = check_auto_work_tasks(&config, &transcript);
        assert!(result.is_none()); // No tasks, shouldn't block
        assert_eq!(reason, "no ready tasks");
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

        let (result, reason) = check_auto_work_tasks(&config, &transcript);
        assert!(result.is_some()); // Has tasks and user is idle - should block
        assert_eq!(reason, "prompting to work on tasks");
        let result = result.unwrap();
        assert!(!result.allow_stop);
        assert!(result.messages.iter().any(|m| m.contains("Open Work Items")));
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

        let (result, reason) = check_auto_work_tasks(&config, &transcript);
        assert!(result.is_some()); // No timestamp = very idle, should block
        assert_eq!(reason, "prompting to work on tasks");
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
        assert!(result.messages.iter().any(|m| m.contains("Open Work Items")));
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
        assert!(result.messages.iter().any(|m| m.contains("Open Work Items")));
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
    fn test_emergency_stop_exit_when_active() {
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let config =
            StopHookConfig { base_dir: Some(dir.path().to_path_buf()), ..Default::default() };

        // Set emergency stop marker
        crate::session::set_emergency_stop(dir.path()).unwrap();
        assert!(crate::session::is_emergency_stop_active(dir.path()));

        // Should allow stop
        let result = check_emergency_stop_exit(&config);
        assert!(result.is_some());
        let result = result.unwrap();
        assert!(result.allow_stop);

        // Marker should be cleared
        assert!(!crate::session::is_emergency_stop_active(dir.path()));
    }

    #[test]
    fn test_emergency_stop_exit_when_not_active() {
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let config =
            StopHookConfig { base_dir: Some(dir.path().to_path_buf()), ..Default::default() };

        // No emergency stop marker
        let result = check_emergency_stop_exit(&config);
        assert!(result.is_none());
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
            check_interactive_question(&transcript_info, &sub_agent, &StopHookConfig::default());
        assert!(result.is_none());
    }

    #[test]
    fn test_check_interactive_question_not_a_question() {
        let transcript_info = TranscriptInfo {
            last_assistant_output: Some("This is just a statement.".to_string()),
            last_user_message_time: None,
            has_api_error: false,
            consecutive_api_errors: 0,
            last_error_is_overloaded: false,
            has_modifying_tool_use: false,
            has_modifying_tool_use_since_user: false,
            first_user_message: None,
            last_user_message: None,
        };
        let sub_agent = MockSubAgent::new();

        let result =
            check_interactive_question(&transcript_info, &sub_agent, &StopHookConfig::default());
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
            last_error_is_overloaded: false,
            has_modifying_tool_use: false,
            has_modifying_tool_use_since_user: false,
            first_user_message: None,
            last_user_message: None,
        };
        let sub_agent = MockSubAgent::new();

        let result =
            check_interactive_question(&transcript_info, &sub_agent, &StopHookConfig::default());
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
            last_error_is_overloaded: false,
            has_modifying_tool_use: false,
            has_modifying_tool_use_since_user: false,
            first_user_message: None,
            last_user_message: None,
        };
        let sub_agent = MockSubAgent::new();

        let result =
            check_interactive_question(&transcript_info, &sub_agent, &StopHookConfig::default());
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
            last_error_is_overloaded: false,
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
            check_interactive_question(&transcript_info, &sub_agent, &StopHookConfig::default());
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
            last_error_is_overloaded: false,
            has_modifying_tool_use: false,
            has_modifying_tool_use_since_user: false,
            first_user_message: None,
            last_user_message: None,
        };
        let mut sub_agent = MockSubAgent::new();
        sub_agent.expect_question_decision(SubAgentDecision::Answer("Use approach A".to_string()));

        let result =
            check_interactive_question(&transcript_info, &sub_agent, &StopHookConfig::default());
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
            last_error_is_overloaded: false,
            has_modifying_tool_use: false,
            has_modifying_tool_use_since_user: false,
            first_user_message: None,
            last_user_message: None,
        };
        let mut sub_agent = MockSubAgent::new();
        sub_agent.expect_question_decision(SubAgentDecision::Continue);

        let result =
            check_interactive_question(&transcript_info, &sub_agent, &StopHookConfig::default());
        assert!(result.is_none());
    }

    #[test]
    fn test_check_interactive_question_subagent_error_allows_stop() {
        use crate::testing::FailingSubAgent;
        use chrono::{Duration, Utc};

        let transcript_info = TranscriptInfo {
            last_assistant_output: Some("Which approach should I use?".to_string()),
            last_user_message_time: Some(Utc::now() - Duration::minutes(1)),
            has_api_error: false,
            consecutive_api_errors: 0,
            last_error_is_overloaded: false,
            has_modifying_tool_use: false,
            has_modifying_tool_use_since_user: false,
            first_user_message: None,
            last_user_message: None,
        };
        let sub_agent = FailingSubAgent::new("command timed out");

        let result =
            check_interactive_question(&transcript_info, &sub_agent, &StopHookConfig::default());
        assert!(result.is_some(), "Should allow stop when sub-agent fails");
        let result = result.unwrap();
        assert!(result.allow_stop, "Should allow stop on sub-agent error");
        assert!(
            result.messages.iter().any(|m| m.contains("Sub-agent failed")),
            "Should mention sub-agent failure in messages"
        );
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
    fn test_emergency_stop_allows_stop_in_run_stop_hook() {
        use tempfile::TempDir;

        let transcript_file = create_transcript_with_output("some output");
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

        // Set emergency stop marker
        crate::session::set_emergency_stop(dir.path()).unwrap();

        let result = run_stop_hook(&input, &config, &runner, &sub_agent).unwrap();
        // Emergency stop should allow stopping
        assert!(result.allow_stop);
        // Marker should be cleared
        assert!(!crate::session::is_emergency_stop_active(dir.path()));
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

        // Set work item reminded marker (simulating that reminder already fired)
        session::set_work_item_reminded(base).unwrap();

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

        // Set work item reminded marker (simulating that reminder already fired)
        session::set_work_item_reminded(base).unwrap();

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
    fn test_check_commit_push_question_commit_now() {
        assert_eq!(
            check_commit_push_question("Would you like me to commit these changes now?"),
            Some("Yes, please commit these changes.".to_string())
        );
        assert_eq!(
            check_commit_push_question("Would you like me to commit now?"),
            Some("Yes, please commit these changes.".to_string())
        );
        assert_eq!(
            check_commit_push_question("Shall I commit these changes now?"),
            Some("Yes, please commit these changes.".to_string())
        );
        assert_eq!(
            check_commit_push_question("Should I commit now?"),
            Some("Yes, please commit these changes.".to_string())
        );
    }

    #[test]
    fn test_check_commit_push_question_push_now() {
        assert_eq!(
            check_commit_push_question("Would you like me to push these changes now?"),
            Some("Yes, please push.".to_string())
        );
        assert_eq!(
            check_commit_push_question("Would you like me to push now?"),
            Some("Yes, please push.".to_string())
        );
        assert_eq!(
            check_commit_push_question("Should I push now?"),
            Some("Yes, please push.".to_string())
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
        // When there are modifications, the first stop prompts work item reminder,
        // second prompts reflection, third stop (with reflection marker set) allows.
        let dir = tempfile::TempDir::new().unwrap();
        let base = dir.path();

        // Set markers (simulating third stop after work item reminder and reflection)
        session::set_work_item_reminded(base).unwrap();
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

        // Set markers since modifying tools trigger work item reminder then reflection
        session::set_work_item_reminded(base).unwrap();
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

    // ========== Commit/Push Keyword Guard Tests ==========

    #[test]
    fn test_contains_commit_push_keywords() {
        assert!(contains_commit_push_keywords("Can you commit this?"));
        assert!(contains_commit_push_keywords("Please COMMIT the changes"));
        assert!(contains_commit_push_keywords("Should I push?"));
        assert!(contains_commit_push_keywords("PUSH to remote?"));
        assert!(contains_commit_push_keywords("commit and push?"));
        assert!(!contains_commit_push_keywords("What does this function do?"));
        assert!(!contains_commit_push_keywords("How does the API work?"));
    }

    #[test]
    fn test_simple_question_with_commit_keyword_no_fast_path() {
        // "Can you commit this?" is a simple question but should NOT fast-path
        // because it contains commit/push keywords — must fall through to auto-confirm.
        let dir = tempfile::TempDir::new().unwrap();
        let base = dir.path();

        let mut runner = MockCommandRunner::new();
        // Git status checks needed because fast path falls through
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

        let transcript_content = r#"{"type": "user", "message": {"role": "user", "content": "Can you commit this?"}}
{"type": "assistant", "message": {"content": [{"type": "text", "text": "Sure, I'll commit it."}]}}
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

        // Should allow (git is clean, no commit/push question from assistant),
        // but NOT via the simple Q&A fast path — it fell through to normal checks.
        assert!(result.allow_stop);
    }

    #[test]
    fn test_simple_question_with_push_keyword_no_fast_path() {
        // "Should I push?" should NOT fast-path due to push keyword.
        let dir = tempfile::TempDir::new().unwrap();
        let base = dir.path();

        let mut runner = MockCommandRunner::new();
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

        let transcript_content = r#"{"type": "user", "message": {"role": "user", "content": "Should I push?"}}
{"type": "assistant", "message": {"content": [{"type": "text", "text": "Yes, you should push."}]}}
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

        // Fell through to normal checks, git is clean, so allows stop
        assert!(result.allow_stop);
    }

    #[test]
    fn test_simple_question_with_uncommitted_changes_no_fast_path() {
        // A simple question like "What does this do?" should NOT fast-path
        // when the repo has uncommitted changes — work needs landing.
        let dir = tempfile::TempDir::new().unwrap();
        let base = dir.path();

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

        let mut runner = MockCommandRunner::new();
        // First check_uncommitted_changes (simple Q&A fast path guard)
        runner.expect("git", &["diff", "--stat"], has_changes.clone());
        runner.expect("git", &["diff", "--name-only"], file_list.clone());
        runner.expect("git", &["diff", "--cached", "--stat"], empty_success.clone());
        runner.expect(
            "git",
            &["ls-files", "--others", "--exclude-standard"],
            empty_success.clone(),
        );
        runner.expect("git", &["rev-list", "--count", "@{upstream}..HEAD"], zero_commits.clone());

        // Second check_uncommitted_changes (Tier 4 main check)
        runner.expect("git", &["diff", "--stat"], has_changes);
        runner.expect("git", &["diff", "--name-only"], file_list);
        runner.expect("git", &["diff", "--cached", "--stat"], empty_success.clone());
        runner.expect("git", &["ls-files", "--others", "--exclude-standard"], empty_success);
        runner.expect("git", &["rev-list", "--count", "@{upstream}..HEAD"], zero_commits);

        let sub_agent = MockSubAgent::new();

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
            git_repo: true,
            base_dir: Some(base.to_path_buf()),
            ..Default::default()
        };

        let result = run_stop_hook(&input, &config, &runner, &sub_agent).unwrap();

        // Should block because of uncommitted changes, NOT fast-path
        assert!(!result.allow_stop);
        assert!(result.messages.iter().any(|m| m.contains("Uncommitted Changes")));
    }

    #[test]
    fn test_simple_question_with_unpushed_commits_no_fast_path() {
        // A simple question should NOT fast-path when there are unpushed commits
        // and require_push is enabled.
        let dir = tempfile::TempDir::new().unwrap();
        let base = dir.path();

        let empty_success =
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() };
        let one_commit =
            CommandOutput { exit_code: 0, stdout: "1\n".to_string(), stderr: String::new() };

        let mut runner = MockCommandRunner::new();
        // First check_uncommitted_changes (simple Q&A fast path guard)
        runner.expect("git", &["diff", "--stat"], empty_success.clone());
        runner.expect("git", &["diff", "--cached", "--stat"], empty_success.clone());
        runner.expect(
            "git",
            &["ls-files", "--others", "--exclude-standard"],
            empty_success.clone(),
        );
        runner.expect("git", &["rev-list", "--count", "@{upstream}..HEAD"], one_commit.clone());

        // Second check_uncommitted_changes (Tier 4 main check)
        runner.expect("git", &["diff", "--stat"], empty_success.clone());
        runner.expect("git", &["diff", "--cached", "--stat"], empty_success.clone());
        runner.expect("git", &["ls-files", "--others", "--exclude-standard"], empty_success);
        runner.expect("git", &["rev-list", "--count", "@{upstream}..HEAD"], one_commit);

        let sub_agent = MockSubAgent::new();

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
            git_repo: true,
            require_push: true,
            base_dir: Some(base.to_path_buf()),
            ..Default::default()
        };

        let result = run_stop_hook(&input, &config, &runner, &sub_agent).unwrap();

        // Should block because of unpushed commits, NOT fast-path
        assert!(!result.allow_stop);
    }

    #[test]
    fn test_simple_question_clean_git_repo_allows_fast_path() {
        // A simple question with a clean git repo (git_repo: true) should still
        // allow the fast path when there are no uncommitted changes or unpushed commits.
        let dir = tempfile::TempDir::new().unwrap();
        let base = dir.path();

        let empty_success =
            CommandOutput { exit_code: 0, stdout: String::new(), stderr: String::new() };
        let zero_commits =
            CommandOutput { exit_code: 0, stdout: "0\n".to_string(), stderr: String::new() };

        let mut runner = MockCommandRunner::new();
        // check_uncommitted_changes (simple Q&A fast path guard) — clean
        runner.expect("git", &["diff", "--stat"], empty_success.clone());
        runner.expect("git", &["diff", "--cached", "--stat"], empty_success.clone());
        runner.expect("git", &["ls-files", "--others", "--exclude-standard"], empty_success);
        runner.expect("git", &["rev-list", "--count", "@{upstream}..HEAD"], zero_commits);

        let sub_agent = MockSubAgent::new();

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
            git_repo: true,
            base_dir: Some(base.to_path_buf()),
            ..Default::default()
        };

        let result = run_stop_hook(&input, &config, &runner, &sub_agent).unwrap();

        // Clean git repo + simple question = fast path should allow stop
        assert!(result.allow_stop, "Simple question with clean git repo should allow stop");
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
        assert!(result.messages.iter().any(|m| m.contains("Requested Work Items Incomplete")));
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
        assert!(result.messages.iter().any(|m| m.contains("Requested Work Items Incomplete")));
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
            result.messages.iter().any(|m| m.contains("Requested Work Items Incomplete")),
            "Expected 'Requested Work Items Incomplete' message but got: {:?}",
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
        assert!(result.messages.iter().any(|m| m.contains("Requested Work Items Incomplete")));
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
            result.messages.iter().any(|m| m.contains("Requested Work Items Incomplete")),
            "Expected 'Requested Work Items Incomplete' message but got: {:?}",
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
    fn test_check_validation_required_timeout_blocks_stop() {
        use crate::testing::TimeoutCommandRunner;

        let dir = TempDir::new().unwrap();
        let config = StopHookConfig {
            base_dir: Some(dir.path().to_path_buf()),
            quality_check_command: Some("just check".to_string()),
            ..Default::default()
        };

        // Set the validation marker
        session::set_needs_validation(dir.path()).unwrap();

        // Use a runner that always returns timeout
        let runner = TimeoutCommandRunner::new(300);
        let result = check_validation_required(&config, &runner).unwrap();

        assert!(result.is_some(), "Expected Some result when timeout occurs");
        let result = result.unwrap();
        assert!(!result.allow_stop, "Should block stop on timeout");
        assert!(
            result.messages.iter().any(|m| m.contains("Quality Check Timeout")),
            "Expected 'Quality Check Timeout' message but got: {:?}",
            result.messages
        );
        assert!(
            result.messages.iter().any(|m| m.contains("5 minutes")),
            "Expected '5 minutes' in message but got: {:?}",
            result.messages
        );
    }

    #[test]
    fn test_handle_uncommitted_changes_timeout_shows_message() {
        use crate::git::{GitStatus, UncommittedChanges};
        use crate::testing::{MockSubAgent, TimeoutCommandRunner};

        let dir = TempDir::new().unwrap();
        let config = StopHookConfig {
            base_dir: Some(dir.path().to_path_buf()),
            quality_check_enabled: true,
            quality_check_command: Some("just check".to_string()),
            ..Default::default()
        };

        let git_status = GitStatus {
            uncommitted: UncommittedChanges {
                has_unstaged: true,
                has_staged: false,
                has_untracked: false,
            },
            unstaged_files: vec!["modified.rs".to_string()],
            ..Default::default()
        };

        let transcript_info = TranscriptInfo::default();
        let sub_agent = MockSubAgent::new();
        let runner = TimeoutCommandRunner::new(300);

        let result =
            handle_uncommitted_changes(&git_status, &config, &runner, &transcript_info, &sub_agent)
                .unwrap();

        assert!(!result.allow_stop, "Should block stop");
        assert!(
            result.messages.iter().any(|m| m.contains("Quality Check Timeout")),
            "Expected 'Quality Check Timeout' message but got: {:?}",
            result.messages
        );
        assert!(
            result.messages.iter().any(|m| m.contains("hanging or stuck")),
            "Expected 'hanging or stuck' in message but got: {:?}",
            result.messages
        );
    }

    #[test]
    fn test_check_validation_required_command_error_propagates() {
        use crate::testing::FailingCommandRunner;

        let dir = TempDir::new().unwrap();
        let config = StopHookConfig {
            base_dir: Some(dir.path().to_path_buf()),
            quality_check_command: Some("just check".to_string()),
            ..Default::default()
        };

        // Set the validation marker
        session::set_needs_validation(dir.path()).unwrap();

        // Use a runner that always returns an error (not a timeout)
        let runner = FailingCommandRunner::new("simulated command failure");
        let result = check_validation_required(&config, &runner);

        assert!(result.is_err(), "Expected error to propagate");
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("simulated command failure"),
            "Expected error message to contain 'simulated command failure' but got: {err}"
        );
    }

    #[test]
    fn test_handle_uncommitted_changes_command_error_propagates() {
        use crate::git::{GitStatus, UncommittedChanges};
        use crate::testing::{FailingCommandRunner, MockSubAgent};

        let dir = TempDir::new().unwrap();
        let config = StopHookConfig {
            base_dir: Some(dir.path().to_path_buf()),
            quality_check_enabled: true,
            quality_check_command: Some("just check".to_string()),
            ..Default::default()
        };

        let git_status = GitStatus {
            uncommitted: UncommittedChanges {
                has_unstaged: true,
                has_staged: false,
                has_untracked: false,
            },
            unstaged_files: vec!["modified.rs".to_string()],
            ..Default::default()
        };

        let transcript_info = TranscriptInfo::default();
        let sub_agent = MockSubAgent::new();
        let runner = FailingCommandRunner::new("simulated command failure");

        let result =
            handle_uncommitted_changes(&git_status, &config, &runner, &transcript_info, &sub_agent);

        assert!(result.is_err(), "Expected error to propagate");
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("simulated command failure"),
            "Expected error message to contain 'simulated command failure' but got: {err}"
        );
    }

    #[test]
    fn test_calculate_overloaded_backoff_first_error() {
        let result = calculate_overloaded_backoff(1);
        match result {
            OverloadedBackoffResult::Retry { sleep_secs, error_count } => {
                // First error: 5s base, ±20% jitter (4-6s range)
                assert!((4..=6).contains(&sleep_secs), "Expected 4-6s, got {sleep_secs}");
                assert_eq!(error_count, 1);
            }
            OverloadedBackoffResult::AllowStop { .. } => {
                panic!("Expected Retry, got AllowStop");
            }
        }
    }

    #[test]
    fn test_calculate_overloaded_backoff_second_error() {
        let result = calculate_overloaded_backoff(2);
        match result {
            OverloadedBackoffResult::Retry { sleep_secs, error_count } => {
                // Second error: 10s base, ±20% jitter (8-12s range)
                assert!((8..=12).contains(&sleep_secs), "Expected 8-12s, got {sleep_secs}");
                assert_eq!(error_count, 2);
            }
            OverloadedBackoffResult::AllowStop { .. } => {
                panic!("Expected Retry, got AllowStop");
            }
        }
    }

    #[test]
    fn test_calculate_overloaded_backoff_exceeds_max() {
        // After 6 errors, total wait is 5*(2^6-1) = 315s > 120s, so should allow stop
        let result = calculate_overloaded_backoff(6);
        match result {
            OverloadedBackoffResult::AllowStop { error_count, total_wait } => {
                assert_eq!(error_count, 6);
                assert!(total_wait > 120, "Expected total_wait > 120, got {total_wait}");
            }
            OverloadedBackoffResult::Retry { .. } => {
                panic!("Expected AllowStop, got Retry");
            }
        }
    }

    #[test]
    fn test_calculate_overloaded_backoff_capped_at_120s() {
        // At 5 errors, base backoff would be 5*16=80s, but total wait is 5*(2^5-1)=155s > 120s
        // Actually, let's verify what happens at 5 errors:
        // backoff = min(5*16, 120) = 80
        // total_wait = 5*(2^5-1) = 155 > 120, so AllowStop
        let result = calculate_overloaded_backoff(5);
        match result {
            OverloadedBackoffResult::AllowStop { error_count, total_wait } => {
                assert_eq!(error_count, 5);
                assert!(total_wait > 120, "Expected total_wait > 120, got {total_wait}");
            }
            OverloadedBackoffResult::Retry { .. } => {
                panic!("Expected AllowStop at 5 errors due to total wait time");
            }
        }
    }

    #[test]
    fn test_calculate_overloaded_backoff_zero_errors() {
        // Edge case: 0 errors should still return Retry (won't happen in practice)
        let result = calculate_overloaded_backoff(0);
        match result {
            OverloadedBackoffResult::Retry { sleep_secs, error_count } => {
                // 0 errors: 5s * 2^(-1) would underflow, but saturating_sub handles it
                // backoff = min(5*1, 120) = 5 (since 2^0=1 due to saturating_sub)
                assert!(sleep_secs <= 6, "Expected <=6s, got {sleep_secs}");
                assert_eq!(error_count, 0);
            }
            OverloadedBackoffResult::AllowStop { .. } => {
                panic!("Expected Retry, got AllowStop");
            }
        }
    }

    #[test]
    fn test_handle_overloaded_error_retry() {
        // Test the retry path with zero backoff for fast testing
        let transcript_info = TranscriptInfo {
            consecutive_api_errors: 1,
            last_error_is_overloaded: true,
            ..Default::default()
        };
        let config = StopHookConfig::default();
        let backoff = BackoffConfig { base_secs: 0, max_secs: 100 };

        let result = handle_overloaded_error_with_backoff(&transcript_info, &config, &backoff);
        assert!(!result.allow_stop, "Overloaded error should block, not allow stop");
        assert!(result.inject_response.is_some(), "Should inject retry message");
        assert!(
            result.inject_response.as_ref().unwrap().contains("try"),
            "Inject message should ask to retry"
        );
    }

    #[test]
    fn test_handle_overloaded_error_allow_stop() {
        // Test the allow stop path when max retry time exceeded
        let transcript_info = TranscriptInfo {
            consecutive_api_errors: 6,
            last_error_is_overloaded: true,
            ..Default::default()
        };
        let config = StopHookConfig::default();
        // Use small max_secs so total wait exceeds it quickly
        let backoff = BackoffConfig { base_secs: 1, max_secs: 5 };

        let result = handle_overloaded_error_with_backoff(&transcript_info, &config, &backoff);
        assert!(result.allow_stop, "Should allow stop after max retry time exceeded");
        assert!(
            result.messages.iter().any(|m| m.contains("Maximum Retry Time")),
            "Should mention max retry time exceeded"
        );
    }

    #[test]
    fn test_check_api_error_loop_non_overloaded_below_threshold() {
        // Test that non-overloaded errors below threshold don't trigger
        let transcript_info = TranscriptInfo {
            consecutive_api_errors: 0,
            last_error_is_overloaded: false,
            ..Default::default()
        };
        let config = StopHookConfig::default();

        let result = check_api_error_loop(&transcript_info, &config);
        // Zero errors should not trigger anything (threshold is 1)
        assert!(result.is_none());
    }

    #[test]
    fn test_check_api_error_loop_non_overloaded_at_threshold() {
        // Test that non-overloaded errors at threshold trigger allow stop
        let transcript_info = TranscriptInfo {
            consecutive_api_errors: 1,
            last_error_is_overloaded: false,
            ..Default::default()
        };
        let config = StopHookConfig::default();

        let result = check_api_error_loop(&transcript_info, &config);
        // Single non-overloaded error should trigger allow stop (threshold is 1)
        assert!(result.is_some());
        let result = result.unwrap();
        assert!(result.allow_stop, "Non-overloaded API error at threshold should allow stop");
    }

    #[test]
    fn test_check_api_error_loop_overloaded_retry() {
        // Test that overloaded errors trigger retry via check_api_error_loop path
        let transcript_info = TranscriptInfo {
            consecutive_api_errors: 1,
            last_error_is_overloaded: true,
            ..Default::default()
        };
        let config = StopHookConfig::default();
        let backoff = BackoffConfig { base_secs: 0, max_secs: 100 };

        let result = check_api_error_loop_with_backoff(&transcript_info, &config, &backoff);
        assert!(result.is_some());
        let result = result.unwrap();
        assert!(!result.allow_stop, "Overloaded error should block stop and prompt retry");
        assert!(result.inject_response.is_some(), "Should inject retry prompt message");
    }

    #[test]
    fn test_calculate_overloaded_backoff_jitter_positive() {
        // Run multiple times to statistically exercise jitter branches
        // The jitter is ±20%, so with base 10s (2nd error), range is 8-12s
        for _ in 0..10 {
            let result = calculate_overloaded_backoff(2);
            if let OverloadedBackoffResult::Retry { sleep_secs, .. } = result {
                // Should be in range 8-12
                assert!((8..=12).contains(&sleep_secs), "Got {sleep_secs}");
            }
        }
    }

    // -- Single work item mode tests --

    #[test]
    fn test_check_single_work_item_complete_allows_when_done() {
        use crate::tasks::{Priority, SqliteTaskStore, Status, TaskStore, TaskUpdate};

        let dir = TempDir::new().unwrap();
        let store = SqliteTaskStore::for_project(dir.path()).unwrap();
        let task = store.create_task("Task", "Do it", Priority::Medium).unwrap();
        store
            .update_task(
                &task.id,
                TaskUpdate { status: Some(Status::Complete), ..Default::default() },
            )
            .unwrap();

        let config = StopHookConfig {
            base_dir: Some(dir.path().to_path_buf()),
            single_work_item_id: Some(task.id.clone()),
            ..Default::default()
        };
        let result = check_single_work_item_complete(&config, &task.id);

        assert!(result.is_none(), "Should allow stop when item is complete");
    }

    #[test]
    fn test_check_single_work_item_complete_blocks_when_incomplete() {
        use crate::tasks::{Priority, SqliteTaskStore, TaskStore};

        let dir = TempDir::new().unwrap();
        let store = SqliteTaskStore::for_project(dir.path()).unwrap();
        let task = store.create_task("Task", "Do it", Priority::Medium).unwrap();

        let config = StopHookConfig {
            base_dir: Some(dir.path().to_path_buf()),
            single_work_item_id: Some(task.id.clone()),
            ..Default::default()
        };
        let result = check_single_work_item_complete(&config, &task.id);

        assert!(result.is_some(), "Should block stop when item is incomplete");
        let result = result.unwrap();
        assert!(!result.allow_stop);
        let joined = result.messages.join("\n");
        assert!(joined.contains("Single Work Item Incomplete"), "messages: {joined}");
        assert!(joined.contains(&task.id), "messages: {joined}");
    }

    #[test]
    fn test_check_requested_tasks_block_dispatches_to_single_item() {
        use crate::tasks::{Priority, SqliteTaskStore, TaskStore};
        use crate::transcript::TranscriptInfo;

        let dir = TempDir::new().unwrap();
        let store = SqliteTaskStore::for_project(dir.path()).unwrap();
        let task = store.create_task("Task", "Do it", Priority::Medium).unwrap();

        let config = StopHookConfig {
            base_dir: Some(dir.path().to_path_buf()),
            single_work_item_id: Some(task.id),
            ..Default::default()
        };
        let transcript_info = TranscriptInfo { has_modifying_tool_use: true, ..Default::default() };
        let result = check_requested_tasks_block(&transcript_info, &config);

        // Item is open so it blocks
        assert!(result.is_some());
        let joined = result.unwrap().messages.join("\n");
        assert!(joined.contains("Single Work Item Incomplete"), "messages: {joined}");
    }

    #[test]
    fn test_check_reflection_marker_blocks_for_incomplete_single_item() {
        use crate::tasks::{Priority, SqliteTaskStore, TaskStore};

        let dir = TempDir::new().unwrap();
        let store = SqliteTaskStore::for_project(dir.path()).unwrap();
        let task = store.create_task("Task", "Do it", Priority::Medium).unwrap();

        // Set reflection marker so the function proceeds
        session::set_reflect_marker(dir.path()).unwrap();

        // Task is still open (incomplete)
        let config = StopHookConfig {
            base_dir: Some(dir.path().to_path_buf()),
            single_work_item_id: Some(task.id),
            ..Default::default()
        };
        let transcript_info = TranscriptInfo::default();
        let sub_agent = MockSubAgent::new();
        let result =
            check_reflection_marker_allow(&config, "test-session", &transcript_info, &sub_agent);

        // Item is incomplete, so reflection should block
        assert!(result.is_some());
        let result = result.unwrap();
        assert!(!result.allow_stop);
        let joined = result.messages.join("\n");
        assert!(joined.contains("Single Work Item Incomplete"), "messages: {joined}");
    }

    #[test]
    fn test_check_reflection_marker_dispatches_to_single_item() {
        use crate::tasks::{Priority, SqliteTaskStore, Status, TaskStore, TaskUpdate};

        let dir = TempDir::new().unwrap();
        let store = SqliteTaskStore::for_project(dir.path()).unwrap();
        let task = store.create_task("Task", "Do it", Priority::Medium).unwrap();

        // Set reflection marker so the function proceeds
        session::set_reflect_marker(dir.path()).unwrap();

        // Complete the task
        store
            .update_task(
                &task.id,
                TaskUpdate { status: Some(Status::Complete), ..Default::default() },
            )
            .unwrap();

        let config = StopHookConfig {
            base_dir: Some(dir.path().to_path_buf()),
            single_work_item_id: Some(task.id),
            ..Default::default()
        };
        let transcript_info = TranscriptInfo::default();
        let sub_agent = MockSubAgent::new();
        let result =
            check_reflection_marker_allow(&config, "test-session", &transcript_info, &sub_agent);

        // Item is complete, so reflection should allow
        assert!(result.is_some());
        assert!(result.unwrap().allow_stop);
    }

    #[test]
    fn test_work_item_reminder_blocks_on_first_stop() {
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
        assert!(result.messages.iter().any(|m| m.contains("Follow-Up Work Items")));
        // Marker should be set
        assert!(session::has_work_item_reminded(base));
    }

    #[test]
    fn test_work_item_reminder_passes_through_on_second_stop() {
        use std::io::Write;
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // Set the work item reminded marker (simulating first stop already happened)
        session::set_work_item_reminded(base).unwrap();

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

        // Second stop: work item reminder passes through, reflection fires instead
        let result = run_stop_hook(&input, &config, &runner, &sub_agent).unwrap();
        assert!(!result.allow_stop);
        assert!(result.messages.iter().any(|m| m.contains("Task Completion Check")));
    }

    #[test]
    fn test_work_item_reminder_skipped_without_modifying_tools() {
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
        // Marker should NOT be set since no modifying tools
        assert!(!session::has_work_item_reminded(base));
    }

    #[test]
    fn test_work_item_reminder_three_stop_integration() {
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

        // First stop: work item reminder fires
        let result = run_stop_hook(&input, &config, &runner, &sub_agent).unwrap();
        assert!(!result.allow_stop);
        assert!(result.messages.iter().any(|m| m.contains("Follow-Up Work Items")));
        assert!(session::has_work_item_reminded(base));

        // Second stop: work item reminder passes through, reflection fires
        let result = run_stop_hook(&input, &config, &runner, &sub_agent).unwrap();
        assert!(!result.allow_stop);
        assert!(result.messages.iter().any(|m| m.contains("Task Completion Check")));
        assert!(session::has_reflect_marker(base));

        // Third stop: reflection marker allows stop
        let result = run_stop_hook(&input, &config, &runner, &sub_agent).unwrap();
        assert!(result.allow_stop);
        assert!(!session::has_reflect_marker(base));
        // work_item_reminded marker persists (set once, never cleared by stop hook)
        assert!(session::has_work_item_reminded(base));
    }

    #[test]
    fn test_truncate_message_short_message() {
        let result = truncate_message("Hello world", 200);
        assert_eq!(result, "Hello world");
    }

    #[test]
    fn test_truncate_message_long_message() {
        let long = "a ".repeat(200);
        let result = truncate_message(&long, 50);
        assert!(result.len() <= 53); // 50 + "..."
        assert!(result.ends_with("..."));
    }

    #[test]
    fn test_truncate_message_collapses_whitespace() {
        let result = truncate_message("Hello   world\n\nnewline\there", 200);
        assert_eq!(result, "Hello world newline here");
    }

    #[test]
    fn test_truncate_message_exact_boundary() {
        // Exactly at the limit
        let msg = "a".repeat(50);
        let result = truncate_message(&msg, 50);
        assert_eq!(result, msg); // No truncation
    }

    #[test]
    fn test_truncate_message_multibyte_char_boundary() {
        // "é" is 2 bytes in UTF-8. Place it so truncation lands mid-character.
        // 49 'a' chars + "é" (2 bytes) = 51 bytes total, max_len = 50
        // Truncation at byte 50 is mid-char, so it backs up to 49.
        let msg = format!("{}é more text", "a".repeat(49));
        let result = truncate_message(&msg, 50);
        assert!(result.ends_with("..."));
        // Should truncate cleanly before the multi-byte char
        assert!(!result.contains("é"), "should not contain partial multi-byte char");
    }

    #[test]
    fn test_reflection_prompt_includes_user_messages() {
        use std::io::Write;
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // Set work item reminded marker
        session::set_work_item_reminded(base).unwrap();

        // Create transcript path first (used as session_id by run_stop_hook)
        let transcript_path = base.join("transcript.jsonl");
        let session_id = transcript_path.to_string_lossy().to_string();

        // Record user messages using transcript path as session_id
        tasks::record_user_message(
            base,
            "Fix the login bug",
            "opening prompt",
            Some(&session_id),
            &session_id,
        );
        tasks::record_user_message(
            base,
            "Also add tests",
            "follow-up",
            Some(&session_id),
            &session_id,
        );

        // Write transcript content with modifying tool use
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
        let input =
            crate::hooks::HookInput { transcript_path: Some(session_id), ..Default::default() };
        let config = StopHookConfig {
            git_repo: false,
            base_dir: Some(base.to_path_buf()),
            ..Default::default()
        };

        let result = run_stop_hook(&input, &config, &runner, &sub_agent).unwrap();
        assert!(!result.allow_stop);
        let joined = result.messages.join("\n");
        assert!(joined.contains("Task Completion Check"), "messages: {joined}");
        assert!(joined.contains("Fix the login bug"), "messages: {joined}");
        assert!(joined.contains("Also add tests"), "messages: {joined}");
        assert!(joined.contains("opening prompt"), "messages: {joined}");
        assert!(joined.contains("follow-up"), "messages: {joined}");
        assert!(joined.contains("verify that ALL user messages"), "messages: {joined}");
    }

    #[test]
    fn test_reflection_prompt_includes_pre_compaction_note() {
        use std::io::Write;
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // Set work item reminded marker
        session::set_work_item_reminded(base).unwrap();

        // Create transcript path first (used as session_id by run_stop_hook)
        let transcript_path = base.join("transcript.jsonl");
        let session_id = transcript_path.to_string_lossy().to_string();

        // Record a message and mark it as pre-compaction
        tasks::record_user_message(
            base,
            "Original request",
            "opening prompt",
            Some(&session_id),
            &session_id,
        );
        tasks::mark_pre_compaction_messages(base, &session_id);

        // Add a post-compaction message
        tasks::record_user_message(
            base,
            "New request after compaction",
            "follow-up",
            Some(&session_id),
            &session_id,
        );

        // Write transcript content with modifying tool use
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
        let input =
            crate::hooks::HookInput { transcript_path: Some(session_id), ..Default::default() };
        let config = StopHookConfig {
            git_repo: false,
            base_dir: Some(base.to_path_buf()),
            ..Default::default()
        };

        let result = run_stop_hook(&input, &config, &runner, &sub_agent).unwrap();
        assert!(!result.allow_stop);
        let joined = result.messages.join("\n");
        assert!(
            joined.contains("before context compaction"),
            "should note pre-compaction: {joined}"
        );
        assert!(joined.contains("Original request"), "messages: {joined}");
        assert!(joined.contains("New request after compaction"), "messages: {joined}");
    }

    #[test]
    fn test_reflection_marker_allow_clears_user_messages() {
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let base = dir.path();
        let session_id = "test-session";

        // Record a user message
        tasks::record_user_message(base, "Do something", "opening prompt", None, session_id);

        // Set reflection marker
        session::set_reflect_marker(base).unwrap();

        let config = StopHookConfig {
            git_repo: false,
            base_dir: Some(base.to_path_buf()),
            ..Default::default()
        };
        let transcript_info = TranscriptInfo::default();
        let sub_agent = MockSubAgent::new();

        let result =
            check_reflection_marker_allow(&config, session_id, &transcript_info, &sub_agent);
        assert!(result.is_some());
        assert!(result.unwrap().allow_stop);

        // User messages should be cleared after successful reflection
        let messages = tasks::get_session_user_messages(base, session_id);
        assert!(messages.is_empty(), "messages should be cleared after reflection");
    }

    #[test]
    fn test_reflection_subagent_says_incomplete_blocks_and_creates_work_items() {
        use crate::tasks::{SqliteTaskStore, TaskFilter, TaskStore};
        use crate::traits::ReflectionDecision;
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let base = dir.path();
        let session_id = "test-session";

        // Ensure task store exists
        let _store = SqliteTaskStore::for_project(base).unwrap();

        // Record a user message
        tasks::record_user_message(base, "Fix the login bug", "opening prompt", None, session_id);

        // Set reflection marker
        session::set_reflect_marker(base).unwrap();

        let config = StopHookConfig {
            git_repo: false,
            base_dir: Some(base.to_path_buf()),
            ..Default::default()
        };

        // Transcript has assistant output (the reflection)
        let transcript_info = TranscriptInfo {
            last_assistant_output: Some("I still need to fix X and add Y".to_string()),
            ..Default::default()
        };

        let mut sub_agent = MockSubAgent::new();
        sub_agent.expect_reflection(ReflectionDecision::Incomplete {
            items: vec!["Fix X".to_string(), "Add Y".to_string()],
        });

        let result =
            check_reflection_marker_allow(&config, session_id, &transcript_info, &sub_agent);

        // Should block
        assert!(result.is_some());
        let result = result.unwrap();
        assert!(!result.allow_stop);
        let joined = result.messages.join("\n");
        assert!(joined.contains("Incomplete Work Detected"), "messages: {joined}");
        assert!(joined.contains("Fix X"), "messages: {joined}");
        assert!(joined.contains("Add Y"), "messages: {joined}");

        // Reflect marker should be re-set for next evaluation
        assert!(session::has_reflect_marker(base));

        // Work items should be created in the task store
        let store = SqliteTaskStore::for_project(base).unwrap();
        let all_tasks = store.list_tasks(TaskFilter::default()).unwrap();
        assert_eq!(all_tasks.len(), 2, "expected 2 tasks created");
        let titles: Vec<&str> = all_tasks.iter().map(|t| t.title.as_str()).collect();
        assert!(titles.contains(&"Fix X"));
        assert!(titles.contains(&"Add Y"));
    }

    #[test]
    fn test_format_work_list_with_created_titles() {
        let items = vec!["Fix X".to_string(), "Add Y".to_string()];
        let created = vec!["- [abc] Fix X".to_string(), "- [def] Add Y".to_string()];
        let result = format_work_list(&items, &created);
        assert_eq!(result, "- [abc] Fix X\n- [def] Add Y");
    }

    #[test]
    fn test_format_work_list_fallback_without_ids() {
        let items = vec!["Fix X".to_string(), "Add Y".to_string()];
        let created: Vec<String> = vec![];
        let result = format_work_list(&items, &created);
        assert_eq!(result, "- Fix X\n- Add Y");
    }

    #[test]
    fn test_reflection_subagent_says_complete_allows_stop() {
        use crate::traits::ReflectionDecision;
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let base = dir.path();
        let session_id = "test-session";

        // Set reflection marker
        session::set_reflect_marker(base).unwrap();

        let config = StopHookConfig {
            git_repo: false,
            base_dir: Some(base.to_path_buf()),
            ..Default::default()
        };

        let transcript_info = TranscriptInfo {
            last_assistant_output: Some("All work is complete".to_string()),
            ..Default::default()
        };

        let mut sub_agent = MockSubAgent::new();
        sub_agent.expect_reflection(ReflectionDecision::Complete);

        let result =
            check_reflection_marker_allow(&config, session_id, &transcript_info, &sub_agent);

        // Should allow
        assert!(result.is_some());
        assert!(result.unwrap().allow_stop);
        // Reflect marker should NOT be re-set
        assert!(!session::has_reflect_marker(base));
    }

    #[test]
    fn test_reflection_subagent_failure_allows_stop() {
        use crate::testing::FailingSubAgent;
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let base = dir.path();
        let session_id = "test-session";

        // Set reflection marker
        session::set_reflect_marker(base).unwrap();

        let config = StopHookConfig {
            git_repo: false,
            base_dir: Some(base.to_path_buf()),
            ..Default::default()
        };

        let transcript_info = TranscriptInfo {
            last_assistant_output: Some("Some reflection output".to_string()),
            ..Default::default()
        };

        let sub_agent = FailingSubAgent::new("sub-agent failed");

        let result =
            check_reflection_marker_allow(&config, session_id, &transcript_info, &sub_agent);

        // Sub-agent error should fall through to allow (fail-open)
        assert!(result.is_some());
        assert!(result.unwrap().allow_stop);
    }

    #[test]
    fn test_reflection_no_assistant_output_allows_stop() {
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let base = dir.path();
        let session_id = "test-session";

        // Set reflection marker
        session::set_reflect_marker(base).unwrap();

        let config = StopHookConfig {
            git_repo: false,
            base_dir: Some(base.to_path_buf()),
            ..Default::default()
        };

        // No assistant output — sub-agent evaluation is skipped
        let transcript_info = TranscriptInfo::default();
        let sub_agent = MockSubAgent::new();

        let result =
            check_reflection_marker_allow(&config, session_id, &transcript_info, &sub_agent);

        // Should allow (no output to evaluate)
        assert!(result.is_some());
        assert!(result.unwrap().allow_stop);
    }

    #[test]
    fn test_reflection_incomplete_then_complete_two_stops() {
        use crate::tasks::SqliteTaskStore;
        use crate::traits::ReflectionDecision;
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let base = dir.path();
        let session_id = "test-session";

        // Ensure task store exists
        let _store = SqliteTaskStore::for_project(base).unwrap();

        // Set reflection marker
        session::set_reflect_marker(base).unwrap();

        let config = StopHookConfig {
            git_repo: false,
            base_dir: Some(base.to_path_buf()),
            ..Default::default()
        };

        // First stop: sub-agent says incomplete
        let transcript_info = TranscriptInfo {
            last_assistant_output: Some("Still need to do X".to_string()),
            ..Default::default()
        };

        let mut sub_agent = MockSubAgent::new();
        sub_agent
            .expect_reflection(ReflectionDecision::Incomplete { items: vec!["Do X".to_string()] });

        let result =
            check_reflection_marker_allow(&config, session_id, &transcript_info, &sub_agent);
        assert!(result.is_some());
        assert!(!result.unwrap().allow_stop);
        // Marker should be re-set
        assert!(session::has_reflect_marker(base));

        // Second stop: sub-agent says complete
        let transcript_info2 = TranscriptInfo {
            last_assistant_output: Some("All done now".to_string()),
            ..Default::default()
        };

        let mut sub_agent2 = MockSubAgent::new();
        sub_agent2.expect_reflection(ReflectionDecision::Complete);

        let result =
            check_reflection_marker_allow(&config, session_id, &transcript_info2, &sub_agent2);
        assert!(result.is_some());
        assert!(result.unwrap().allow_stop);
        assert!(!session::has_reflect_marker(base));
    }
}
