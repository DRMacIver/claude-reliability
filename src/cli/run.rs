//! Command execution for the CLI.
//!
//! This module handles running CLI commands and producing output.

use crate::cli::{Command, HowToCommand, QuestionCommand, WorkCommand};
use crate::command::RealCommandRunner;
use crate::config;
use crate::hooks::{
    parse_hook_input, run_post_tool_use, run_pre_tool_use, run_stop_hook,
    run_user_prompt_submit_hook, PostToolUseInput, StopHookConfig,
};
use crate::paths;
use crate::session;
use crate::subagent::RealSubAgent;
use crate::tasks::{
    HowToUpdate, Priority, SqliteTaskStore, Status, TaskFilter, TaskStore, TaskUpdate,
};
use serde::Serialize;
use std::path::Path;
use std::process::ExitCode;

/// Output from running the CLI, with separate stdout and stderr messages.
#[derive(Debug)]
pub struct CliOutput {
    /// Exit code for the process.
    pub exit_code: ExitCode,
    /// Messages to print to stdout.
    pub stdout: Vec<String>,
    /// Messages to print to stderr.
    pub stderr: Vec<String>,
}

/// Default result limit for list/search operations.
const DEFAULT_RESULT_LIMIT: usize = 50;

/// Run a CLI command with the given stdin input.
pub fn run(command: Command, stdin: &str) -> CliOutput {
    // Log hook events for debugging when enabled
    if let Some(hook_type) = command.hook_type() {
        crate::hook_logging::log_hook_event(hook_type, stdin);
    }

    match command {
        Command::Version => run_version(),
        Command::EnsureConfig => run_ensure_config(),
        Command::EnsureGitignore => run_ensure_gitignore(),
        Command::Intro => run_intro(),
        Command::Stop => run_stop_cmd(stdin),
        Command::UserPromptSubmit => run_user_prompt_submit_cmd(stdin),
        Command::PreToolUse => run_pre_tool_use_cmd(stdin),
        Command::PostToolUse => run_post_tool_use_cmd(stdin),
        Command::Work(cmd) => run_work_cmd(cmd),
        Command::Howto(cmd) => run_howto_cmd(cmd),
        Command::Question(cmd) => run_question_cmd(cmd),
        Command::AuditLog { work_id, limit } => run_audit_log(work_id.as_ref(), limit),
        Command::EmergencyStop { explanation } => run_emergency_stop(explanation),
    }
}

// === Utility Commands ===

fn run_version() -> CliOutput {
    CliOutput {
        exit_code: ExitCode::SUCCESS,
        stdout: vec![],
        stderr: vec![format!("claude-reliability v{}", crate::VERSION)],
    }
}

fn run_ensure_config() -> CliOutput {
    let runner = RealCommandRunner::new();
    match config::ensure_config(&runner) {
        Ok(config) => {
            let mut messages =
                vec!["Config ensured at .claude/reliability-config.yaml".to_string()];
            messages.push(format!("  git_repo: {}", config.git_repo));
            if let Some(ref cmd) = config.check_command {
                messages.push(format!("  check_command: {cmd}"));
            } else {
                messages.push("  check_command: (none)".to_string());
            }
            CliOutput { exit_code: ExitCode::SUCCESS, stdout: vec![], stderr: messages }
        }
        Err(e) => CliOutput {
            exit_code: ExitCode::from(1),
            stdout: vec![],
            stderr: vec![format!("Error ensuring config: {e}")],
        },
    }
}

fn run_ensure_gitignore() -> CliOutput {
    match config::ensure_gitignore(Path::new(".")) {
        Ok(modified) => {
            let msg = if modified {
                "Updated .gitignore with claude-reliability entries"
            } else {
                ".gitignore already has required entries"
            };
            CliOutput {
                exit_code: ExitCode::SUCCESS,
                stdout: vec![],
                stderr: vec![msg.to_string()],
            }
        }
        Err(e) => CliOutput {
            exit_code: ExitCode::from(1),
            stdout: vec![],
            stderr: vec![format!("Error updating .gitignore: {e}")],
        },
    }
}

fn run_intro() -> CliOutput {
    use crate::templates;
    use tera::Context;

    let message = templates::render("messages/session_intro.tera", &Context::new())
        .expect("session_intro.tera template should always render");
    CliOutput { exit_code: ExitCode::SUCCESS, stdout: vec![], stderr: vec![message] }
}

// === Hook Commands ===

fn run_stop_cmd(stdin: &str) -> CliOutput {
    let runner = RealCommandRunner::new();
    let sub_agent = RealSubAgent::new(&runner);

    let project_config = match config::ensure_config(&runner) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Warning: Could not load config: {e}");
            config::ProjectConfig::default()
        }
    };

    let config = StopHookConfig {
        git_repo: project_config.git_repo,
        quality_check_enabled: project_config.check_command.is_some(),
        quality_check_command: project_config.check_command,
        require_push: project_config.require_push,
        base_dir: None,
        explain_stops: project_config.explain_stops,
        auto_work_on_tasks: project_config.auto_work_on_tasks,
        auto_work_idle_minutes: project_config.auto_work_idle_minutes,
    };

    let input = match parse_hook_input(stdin) {
        Ok(i) => i,
        Err(e) => {
            return CliOutput {
                exit_code: ExitCode::from(1),
                stdout: vec![],
                stderr: vec![format!("Error parsing hook input: {e}")],
            }
        }
    };

    match run_stop_hook(&input, &config, &runner, &sub_agent) {
        Ok(result) => {
            let exit_code = exit_code_from_i32(result.exit_code);
            let outputs_json = exit_code == ExitCode::SUCCESS;

            if outputs_json && !result.messages.is_empty() {
                let first_msg = &result.messages[0];
                if first_msg.starts_with('{') {
                    CliOutput { exit_code, stdout: result.messages, stderr: vec![] }
                } else {
                    let system_message = result.messages.join("\n");
                    let json = serde_json::json!({"systemMessage": system_message});
                    CliOutput { exit_code, stdout: vec![json.to_string()], stderr: vec![] }
                }
            } else {
                CliOutput { exit_code, stdout: vec![], stderr: result.messages }
            }
        }
        Err(e) => CliOutput {
            exit_code: ExitCode::from(1),
            stdout: vec![],
            stderr: vec![format!("Error running stop hook: {e}")],
        },
    }
}

fn run_user_prompt_submit_cmd(stdin: &str) -> CliOutput {
    use crate::hooks::UserPromptSubmitInput;

    let input: UserPromptSubmitInput =
        serde_json::from_str(stdin).unwrap_or_else(|_| UserPromptSubmitInput::default());

    match run_user_prompt_submit_hook(&input, None) {
        Ok(output) => {
            if output.system_message.is_some() {
                let json = serde_json::to_string(&output).expect("output should serialize");
                CliOutput { exit_code: ExitCode::SUCCESS, stdout: vec![json], stderr: vec![] }
            } else {
                CliOutput { exit_code: ExitCode::SUCCESS, stdout: vec![], stderr: vec![] }
            }
        }
        Err(e) => CliOutput {
            exit_code: ExitCode::from(1),
            stdout: vec![],
            stderr: vec![format!("Error running user-prompt-submit hook: {e}")],
        },
    }
}

fn run_pre_tool_use_cmd(stdin: &str) -> CliOutput {
    let input = match parse_hook_input(stdin) {
        Ok(input) => input,
        Err(e) => {
            return CliOutput {
                exit_code: ExitCode::from(1),
                stdout: vec![],
                stderr: vec![format!("Failed to parse input: {e}")],
            }
        }
    };

    let runner = RealCommandRunner::new();
    let output = run_pre_tool_use(&input, Path::new("."), &runner);
    let json = serde_json::to_string(&output).expect("PreToolUseOutput serialization cannot fail");

    CliOutput { exit_code: ExitCode::SUCCESS, stdout: vec![], stderr: vec![json] }
}

fn run_post_tool_use_cmd(stdin: &str) -> CliOutput {
    let input: PostToolUseInput = match serde_json::from_str(stdin) {
        Ok(input) => input,
        Err(e) => {
            return CliOutput {
                exit_code: ExitCode::from(1),
                stdout: vec![],
                stderr: vec![format!("Failed to parse input: {e}")],
            }
        }
    };

    match run_post_tool_use(&input, Path::new(".")) {
        Ok(()) => CliOutput { exit_code: ExitCode::SUCCESS, stdout: vec![], stderr: vec![] },
        Err(e) => CliOutput { exit_code: ExitCode::from(1), stdout: vec![], stderr: vec![e] },
    }
}

// === Work Commands ===

fn run_work_cmd(cmd: WorkCommand) -> CliOutput {
    let store = match open_store() {
        Ok(s) => s,
        Err(e) => return error_output(e),
    };

    match cmd {
        WorkCommand::Create { title, description, priority } => {
            work_create(&store, &title, &description, priority)
        }
        WorkCommand::Get { id } => work_get(&store, &id),
        WorkCommand::Update { id, title, description, priority, status } => {
            work_update(&store, &id, title, description, priority, status.as_ref())
        }
        WorkCommand::Delete { id } => work_delete(&store, &id),
        WorkCommand::List { status, priority, max_priority, ready_only, limit, offset } => {
            work_list(&store, status.as_ref(), priority, max_priority, ready_only, limit, offset)
        }
        WorkCommand::Search { query, limit } => work_search(&store, &query, limit),
        WorkCommand::Next => work_next(&store),
        WorkCommand::On { id } => work_on(&store, &id),
        WorkCommand::Request { ids } => work_request(&store, &ids),
        WorkCommand::RequestAll => work_request_all(&store),
        WorkCommand::Incomplete => work_incomplete(&store),
        WorkCommand::Blocked => work_blocked(&store),
        WorkCommand::AddDep { id, depends_on } => work_add_dep(&store, &id, &depends_on),
        WorkCommand::RemoveDep { id, depends_on } => work_remove_dep(&store, &id, &depends_on),
        WorkCommand::AddNote { id, content } => work_add_note(&store, &id, &content),
        WorkCommand::Notes { id, limit } => work_notes(&store, &id, limit),
        WorkCommand::LinkHowTo { id, howto_id } => work_link_howto(&store, &id, &howto_id),
        WorkCommand::UnlinkHowTo { id, howto_id } => work_unlink_howto(&store, &id, &howto_id),
    }
}

fn work_create(store: &SqliteTaskStore, title: &str, description: &str, priority: u8) -> CliOutput {
    let priority = match Priority::from_u8(priority) {
        Ok(p) => p,
        Err(e) => return error_output(e.to_string()),
    };

    match store.create_task(title, description, priority) {
        Ok(task) => {
            let output = WorkItemOutput::from_task(&task, vec![], vec![]);
            json_output(&output)
        }
        Err(e) => error_output(e.to_string()),
    }
}

fn work_get(store: &SqliteTaskStore, id: &str) -> CliOutput {
    match store.get_task(id) {
        Ok(Some(task)) => {
            let deps = store.get_dependencies(&task.id).unwrap_or_default();
            let notes = store.get_notes(&task.id).unwrap_or_default();
            let guidance = store.get_task_guidance(&task.id).unwrap_or_default();

            let howtos: Vec<HowToOutput> = guidance
                .iter()
                .filter_map(|id| store.get_howto(id).ok().flatten().map(|h| HowToOutput::from(&h)))
                .collect();

            let output = FullWorkItemOutput {
                task: WorkItemOutput::from_task(&task, deps, guidance),
                notes: notes.into_iter().map(NoteOutput::from).collect(),
                howtos,
            };
            json_output(&output)
        }
        Ok(None) => error_output(format!("Work item not found: {id}")),
        Err(e) => error_output(e.to_string()),
    }
}

fn work_update(
    store: &SqliteTaskStore,
    id: &str,
    title: Option<String>,
    description: Option<String>,
    priority: Option<u8>,
    status: Option<&String>,
) -> CliOutput {
    let priority = match priority.map(Priority::from_u8).transpose() {
        Ok(p) => p,
        Err(e) => return error_output(e.to_string()),
    };

    let status = match status.map(|s| Status::from_str(s)).transpose() {
        Ok(s) => s,
        Err(e) => return error_output(e.to_string()),
    };

    let update =
        TaskUpdate { title, description, priority, status, in_progress: None, requested: None };

    match store.update_task(id, update) {
        Ok(Some(task)) => {
            let deps = store.get_dependencies(&task.id).unwrap_or_default();
            let guidance = store.get_task_guidance(&task.id).unwrap_or_default();
            let output = WorkItemOutput::from_task(&task, deps, guidance);
            json_output(&output)
        }
        Ok(None) => error_output(format!("Work item not found: {id}")),
        Err(e) => error_output(e.to_string()),
    }
}

fn work_delete(store: &SqliteTaskStore, id: &str) -> CliOutput {
    match store.delete_task(id) {
        Ok(true) => success_output(format!("Work item deleted: {id}")),
        Ok(false) => error_output(format!("Work item not found: {id}")),
        Err(e) => error_output(e.to_string()),
    }
}

fn work_list(
    store: &SqliteTaskStore,
    status: Option<&String>,
    priority: Option<u8>,
    max_priority: Option<u8>,
    ready_only: bool,
    limit: Option<usize>,
    offset: Option<usize>,
) -> CliOutput {
    let status = match status.map(|s| Status::from_str(s)).transpose() {
        Ok(s) => s,
        Err(e) => return error_output(e.to_string()),
    };

    let priority = match priority.map(Priority::from_u8).transpose() {
        Ok(p) => p,
        Err(e) => return error_output(e.to_string()),
    };

    let max_priority = match max_priority.map(Priority::from_u8).transpose() {
        Ok(p) => p,
        Err(e) => return error_output(e.to_string()),
    };

    let filter = TaskFilter {
        status,
        priority,
        max_priority,
        ready_only,
        limit: Some(limit.unwrap_or(DEFAULT_RESULT_LIMIT)),
        offset,
    };

    match store.list_tasks(filter) {
        Ok(tasks) => {
            let outputs: Vec<WorkItemSummary> = tasks
                .iter()
                .map(|t| {
                    let blocked_by: Vec<String> = store
                        .get_dependencies(&t.id)
                        .unwrap_or_default()
                        .into_iter()
                        .filter(|dep_id| {
                            store
                                .get_task(dep_id)
                                .ok()
                                .flatten()
                                .is_some_and(|t| t.status != Status::Complete)
                        })
                        .collect();
                    WorkItemSummary::from_task(t, blocked_by)
                })
                .collect();
            json_output(&outputs)
        }
        Err(e) => error_output(e.to_string()),
    }
}

fn work_search(store: &SqliteTaskStore, query: &str, limit: Option<usize>) -> CliOutput {
    match store.search_tasks(query) {
        Ok(tasks) => {
            let max = limit.unwrap_or(DEFAULT_RESULT_LIMIT);
            let outputs: Vec<WorkItemSummary> = tasks
                .iter()
                .take(max)
                .map(|t| {
                    let blocked_by: Vec<String> = store
                        .get_dependencies(&t.id)
                        .unwrap_or_default()
                        .into_iter()
                        .filter(|dep_id| {
                            store
                                .get_task(dep_id)
                                .ok()
                                .flatten()
                                .is_some_and(|t| t.status != Status::Complete)
                        })
                        .collect();
                    WorkItemSummary::from_task(t, blocked_by)
                })
                .collect();
            json_output(&outputs)
        }
        Err(e) => error_output(e.to_string()),
    }
}

fn work_next(store: &SqliteTaskStore) -> CliOutput {
    match store.pick_task() {
        Ok(Some(task)) => {
            let deps = store.get_dependencies(&task.id).unwrap_or_default();
            let notes = store.get_notes(&task.id).unwrap_or_default();
            let guidance = store.get_task_guidance(&task.id).unwrap_or_default();

            let howtos: Vec<HowToOutput> = guidance
                .iter()
                .filter_map(|id| store.get_howto(id).ok().flatten().map(|h| HowToOutput::from(&h)))
                .collect();

            let output = WorkItemSuggestion {
                task: WorkItemOutput::from_task(&task, deps, guidance),
                notes: notes.into_iter().map(NoteOutput::from).collect(),
                howtos,
                message: format!(
                    "Suggested work item: {} (priority: {})",
                    task.title,
                    priority_label(task.priority)
                ),
            };
            json_output(&output)
        }
        Ok(None) => success_output(
            "No work items available. All items are either complete, blocked, or the list is empty."
                .to_string(),
        ),
        Err(e) => error_output(e.to_string()),
    }
}

fn work_on(store: &SqliteTaskStore, id: &str) -> CliOutput {
    let update = TaskUpdate { in_progress: Some(true), ..Default::default() };

    match store.update_task(id, update) {
        Ok(Some(task)) => {
            let deps = store.get_dependencies(&task.id).unwrap_or_default();
            let guidance = store.get_task_guidance(&task.id).unwrap_or_default();
            let output = WorkItemOutput::from_task(&task, deps, guidance);
            json_output(&output)
        }
        Ok(None) => error_output(format!("Work item not found: {id}")),
        Err(e) => error_output(e.to_string()),
    }
}

fn work_request(store: &SqliteTaskStore, ids: &[String]) -> CliOutput {
    let id_refs: Vec<&str> = ids.iter().map(String::as_str).collect();
    match store.request_tasks(&id_refs) {
        Ok(updated) => success_output(format!("Marked {updated} work item(s) as requested.")),
        Err(e) => error_output(e.to_string()),
    }
}

fn work_request_all(store: &SqliteTaskStore) -> CliOutput {
    match store.request_all_open() {
        Ok(updated) => success_output(format!(
            "Marked {updated} work item(s) as requested. Request mode enabled."
        )),
        Err(e) => error_output(e.to_string()),
    }
}

fn work_incomplete(store: &SqliteTaskStore) -> CliOutput {
    match store.get_incomplete_requested_work() {
        Ok(tasks) if tasks.is_empty() => success_output(
            "No incomplete requested work items. The agent may stop when ready.".to_string(),
        ),
        Ok(tasks) => {
            let outputs: Vec<WorkItemSummary> = tasks
                .iter()
                .map(|t| {
                    let blocked_by: Vec<String> = store
                        .get_dependencies(&t.id)
                        .unwrap_or_default()
                        .into_iter()
                        .filter(|dep_id| {
                            store
                                .get_task(dep_id)
                                .ok()
                                .flatten()
                                .is_some_and(|t| t.status != Status::Complete)
                        })
                        .collect();
                    WorkItemSummary::from_task(t, blocked_by)
                })
                .collect();
            json_output(&outputs)
        }
        Err(e) => error_output(e.to_string()),
    }
}

fn work_blocked(store: &SqliteTaskStore) -> CliOutput {
    match store.get_question_blocked_tasks() {
        Ok(tasks) => {
            let outputs: Vec<WorkItemOutput> = tasks
                .iter()
                .map(|t| {
                    let deps = store.get_dependencies(&t.id).unwrap_or_default();
                    let guidance = store.get_task_guidance(&t.id).unwrap_or_default();
                    WorkItemOutput::from_task(t, deps, guidance)
                })
                .collect();
            json_output(&outputs)
        }
        Err(e) => error_output(e.to_string()),
    }
}

fn work_add_dep(store: &SqliteTaskStore, id: &str, depends_on: &str) -> CliOutput {
    match store.add_dependency(id, depends_on) {
        Ok(()) => success_output(format!("Dependency added: {id} now depends on {depends_on}")),
        Err(e) => error_output(e.to_string()),
    }
}

fn work_remove_dep(store: &SqliteTaskStore, id: &str, depends_on: &str) -> CliOutput {
    match store.remove_dependency(id, depends_on) {
        Ok(true) => {
            success_output(format!("Dependency removed: {id} no longer depends on {depends_on}"))
        }
        Ok(false) => error_output("Dependency not found".to_string()),
        Err(e) => error_output(e.to_string()),
    }
}

fn work_add_note(store: &SqliteTaskStore, id: &str, content: &str) -> CliOutput {
    match store.add_note(id, content) {
        Ok(note) => {
            let output = NoteWithWorkItemOutput {
                id: note.id,
                work_item_id: note.task_id,
                content: note.content,
                created_at: note.created_at,
            };
            json_output(&output)
        }
        Err(e) => error_output(e.to_string()),
    }
}

fn work_notes(store: &SqliteTaskStore, id: &str, limit: Option<usize>) -> CliOutput {
    match store.get_notes(id) {
        Ok(notes) => {
            let max = limit.unwrap_or(DEFAULT_RESULT_LIMIT);
            let outputs: Vec<NoteOutput> =
                notes.into_iter().take(max).map(NoteOutput::from).collect();
            json_output(&outputs)
        }
        Err(e) => error_output(e.to_string()),
    }
}

fn work_link_howto(store: &SqliteTaskStore, id: &str, howto_id: &str) -> CliOutput {
    match store.link_task_to_howto(id, howto_id) {
        Ok(()) => success_output(format!("Linked work item {id} to how-to {howto_id}")),
        Err(e) => error_output(e.to_string()),
    }
}

fn work_unlink_howto(store: &SqliteTaskStore, id: &str, howto_id: &str) -> CliOutput {
    match store.unlink_task_from_howto(id, howto_id) {
        Ok(true) => success_output(format!("Unlinked work item {id} from how-to {howto_id}")),
        Ok(false) => {
            error_output(format!("No link found between work item {id} and how-to {howto_id}"))
        }
        Err(e) => error_output(e.to_string()),
    }
}

// === HowTo Commands ===

fn run_howto_cmd(cmd: HowToCommand) -> CliOutput {
    let store = match open_store() {
        Ok(s) => s,
        Err(e) => return error_output(e),
    };

    match cmd {
        HowToCommand::Create { title, instructions } => howto_create(&store, &title, &instructions),
        HowToCommand::Get { id } => howto_get(&store, &id),
        HowToCommand::Update { id, title, instructions } => {
            howto_update(&store, &id, title, instructions)
        }
        HowToCommand::Delete { id } => howto_delete(&store, &id),
        HowToCommand::List => howto_list(&store),
        HowToCommand::Search { query, limit } => howto_search(&store, &query, limit),
    }
}

fn howto_create(store: &SqliteTaskStore, title: &str, instructions: &str) -> CliOutput {
    match store.create_howto(title, instructions) {
        Ok(howto) => json_output(&HowToOutput::from(&howto)),
        Err(e) => error_output(e.to_string()),
    }
}

fn howto_get(store: &SqliteTaskStore, id: &str) -> CliOutput {
    match store.get_howto(id) {
        Ok(Some(howto)) => json_output(&HowToOutput::from(&howto)),
        Ok(None) => error_output(format!("How-to not found: {id}")),
        Err(e) => error_output(e.to_string()),
    }
}

fn howto_update(
    store: &SqliteTaskStore,
    id: &str,
    title: Option<String>,
    instructions: Option<String>,
) -> CliOutput {
    let update = HowToUpdate { title, instructions };
    match store.update_howto(id, update) {
        Ok(Some(howto)) => json_output(&HowToOutput::from(&howto)),
        Ok(None) => error_output(format!("How-to not found: {id}")),
        Err(e) => error_output(e.to_string()),
    }
}

fn howto_delete(store: &SqliteTaskStore, id: &str) -> CliOutput {
    match store.delete_howto(id) {
        Ok(true) => success_output(format!("Deleted how-to: {id}")),
        Ok(false) => error_output(format!("How-to not found: {id}")),
        Err(e) => error_output(e.to_string()),
    }
}

fn howto_list(store: &SqliteTaskStore) -> CliOutput {
    match store.list_howtos() {
        Ok(howtos) => {
            let outputs: Vec<HowToOutput> = howtos.iter().map(HowToOutput::from).collect();
            json_output(&outputs)
        }
        Err(e) => error_output(e.to_string()),
    }
}

fn howto_search(store: &SqliteTaskStore, query: &str, limit: Option<usize>) -> CliOutput {
    match store.search_howtos(query) {
        Ok(howtos) => {
            let max = limit.unwrap_or(DEFAULT_RESULT_LIMIT);
            let outputs: Vec<HowToOutput> =
                howtos.iter().take(max).map(HowToOutput::from).collect();
            json_output(&outputs)
        }
        Err(e) => error_output(e.to_string()),
    }
}

// === Question Commands ===

fn run_question_cmd(cmd: QuestionCommand) -> CliOutput {
    let store = match open_store() {
        Ok(s) => s,
        Err(e) => return error_output(e),
    };

    match cmd {
        QuestionCommand::Create { text } => question_create(&store, &text),
        QuestionCommand::Get { id } => question_get(&store, &id),
        QuestionCommand::Answer { id, answer } => question_answer(&store, &id, &answer),
        QuestionCommand::Delete { id } => question_delete(&store, &id),
        QuestionCommand::List { unanswered_only, limit } => {
            question_list(&store, unanswered_only, limit)
        }
        QuestionCommand::Search { query, limit } => question_search(&store, &query, limit),
        QuestionCommand::Link { work_id, question_id } => {
            question_link(&store, &work_id, &question_id)
        }
        QuestionCommand::Unlink { work_id, question_id } => {
            question_unlink(&store, &work_id, &question_id)
        }
        QuestionCommand::Blocking { work_id } => question_blocking(&store, &work_id),
    }
}

fn question_create(store: &SqliteTaskStore, text: &str) -> CliOutput {
    use crate::traits::{CreateQuestionContext, CreateQuestionDecision, SubAgent as _};

    // Evaluate whether this question can be auto-answered
    let runner = RealCommandRunner::new();
    let sub_agent = RealSubAgent::new(&runner);
    let context = CreateQuestionContext { question_text: text.to_string() };

    match sub_agent.evaluate_create_question(&context) {
        Ok(CreateQuestionDecision::AutoAnswer(answer)) => {
            return success_output(format!(
                "Question auto-answered (no user input needed):\n\n{answer}"
            ));
        }
        Ok(CreateQuestionDecision::Create) | Err(_) => {}
    }

    match store.create_question(text) {
        Ok(question) => json_output(&QuestionOutput::from(&question)),
        Err(e) => error_output(e.to_string()),
    }
}

fn question_get(store: &SqliteTaskStore, id: &str) -> CliOutput {
    match store.get_question(id) {
        Ok(Some(q)) => json_output(&QuestionOutput::from(&q)),
        Ok(None) => error_output(format!("Question not found: {id}")),
        Err(e) => error_output(e.to_string()),
    }
}

fn question_answer(store: &SqliteTaskStore, id: &str, answer: &str) -> CliOutput {
    match store.answer_question(id, answer) {
        Ok(Some(q)) => json_output(&QuestionOutput::from(&q)),
        Ok(None) => error_output(format!("Question not found: {id}")),
        Err(e) => error_output(e.to_string()),
    }
}

fn question_delete(store: &SqliteTaskStore, id: &str) -> CliOutput {
    match store.delete_question(id) {
        Ok(true) => success_output(format!("Deleted question: {id}")),
        Ok(false) => error_output(format!("Question not found: {id}")),
        Err(e) => error_output(e.to_string()),
    }
}

fn question_list(
    store: &SqliteTaskStore,
    unanswered_only: bool,
    limit: Option<usize>,
) -> CliOutput {
    match store.list_questions(unanswered_only) {
        Ok(questions) => {
            let max = limit.unwrap_or(DEFAULT_RESULT_LIMIT);
            let outputs: Vec<QuestionOutput> =
                questions.iter().take(max).map(QuestionOutput::from).collect();
            json_output(&outputs)
        }
        Err(e) => error_output(e.to_string()),
    }
}

fn question_search(store: &SqliteTaskStore, query: &str, limit: Option<usize>) -> CliOutput {
    match store.search_questions(query) {
        Ok(questions) => {
            let max = limit.unwrap_or(DEFAULT_RESULT_LIMIT);
            let outputs: Vec<QuestionOutput> =
                questions.iter().take(max).map(QuestionOutput::from).collect();
            json_output(&outputs)
        }
        Err(e) => error_output(e.to_string()),
    }
}

fn question_link(store: &SqliteTaskStore, work_id: &str, question_id: &str) -> CliOutput {
    match store.link_task_to_question(work_id, question_id) {
        Ok(()) => success_output(format!(
            "Linked work item {work_id} to question {question_id} - item is blocked until question is answered"
        )),
        Err(e) => error_output(e.to_string()),
    }
}

fn question_unlink(store: &SqliteTaskStore, work_id: &str, question_id: &str) -> CliOutput {
    match store.unlink_task_from_question(work_id, question_id) {
        Ok(true) => {
            success_output(format!("Unlinked work item {work_id} from question {question_id}"))
        }
        Ok(false) => error_output(format!(
            "No link found between work item {work_id} and question {question_id}"
        )),
        Err(e) => error_output(e.to_string()),
    }
}

fn question_blocking(store: &SqliteTaskStore, work_id: &str) -> CliOutput {
    match store.get_blocking_questions(work_id) {
        Ok(questions) => {
            let outputs: Vec<QuestionOutput> = questions.iter().map(QuestionOutput::from).collect();
            json_output(&outputs)
        }
        Err(e) => error_output(e.to_string()),
    }
}

// === Audit and Emergency Stop ===

fn run_audit_log(work_id: Option<&String>, limit: Option<usize>) -> CliOutput {
    let store = match open_store() {
        Ok(s) => s,
        Err(e) => return error_output(e),
    };

    let limit = Some(limit.unwrap_or(DEFAULT_RESULT_LIMIT));
    match store.get_audit_log(work_id.map(String::as_str), limit) {
        Ok(entries) => json_output(&entries),
        Err(e) => error_output(e.to_string()),
    }
}

fn run_emergency_stop(explanation: String) -> CliOutput {
    use crate::traits::{EmergencyStopContext, EmergencyStopDecision, SubAgent as _};

    let runner = RealCommandRunner::new();
    let sub_agent = RealSubAgent::new(&runner);
    let base_dir = std::env::current_dir().unwrap_or_default();

    let context = EmergencyStopContext { explanation };

    match sub_agent.evaluate_emergency_stop(&context) {
        Ok(EmergencyStopDecision::Accept(msg)) => {
            if let Err(e) = session::set_emergency_stop(&base_dir) {
                return error_output(e.to_string());
            }

            let message = msg.map_or_else(
                || {
                    "Emergency stop accepted.\n\n\
                     Please explain the problem clearly to the user, then stop."
                        .to_string()
                },
                |m| {
                    format!(
                        "Emergency stop accepted: {m}\n\n\
                         Please explain the problem clearly to the user, then stop."
                    )
                },
            );
            success_output(message)
        }
        Ok(EmergencyStopDecision::Reject(instructions)) => {
            error_output(format!("Emergency stop denied. {instructions}"))
        }
        Err(_) => {
            // On failure, default to accepting (conservative)
            if let Err(e) = session::set_emergency_stop(&base_dir) {
                return error_output(e.to_string());
            }
            success_output(
                "Emergency stop accepted (evaluation unavailable).\n\n\
                 Please explain the problem clearly to the user, then stop."
                    .to_string(),
            )
        }
    }
}

// === Helper Functions ===

fn open_store() -> Result<SqliteTaskStore, String> {
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let db_path = paths::project_db_path(&cwd);
    SqliteTaskStore::new(&db_path).map_err(|e| e.to_string())
}

fn json_output<T: Serialize>(value: &T) -> CliOutput {
    match serde_json::to_string_pretty(value) {
        Ok(json) => CliOutput { exit_code: ExitCode::SUCCESS, stdout: vec![json], stderr: vec![] },
        Err(e) => error_output(e.to_string()),
    }
}

fn success_output(message: String) -> CliOutput {
    CliOutput { exit_code: ExitCode::SUCCESS, stdout: vec![message], stderr: vec![] }
}

fn error_output(message: String) -> CliOutput {
    CliOutput { exit_code: ExitCode::from(1), stdout: vec![], stderr: vec![message] }
}

fn exit_code_from_i32(code: i32) -> ExitCode {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let code_u8 = if code < 0 {
        1u8
    } else if code > 255 {
        255u8
    } else {
        code as u8
    };
    ExitCode::from(code_u8)
}

// === Output Types ===

/// Work item summary for list operations.
#[derive(Debug, Serialize)]
struct WorkItemSummary {
    id: String,
    title: String,
    priority: u8,
    priority_label: &'static str,
    status: String,
    in_progress: bool,
    requested: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    blocked_by: Vec<String>,
}

impl WorkItemSummary {
    fn from_task(task: &crate::tasks::Task, blocked_by: Vec<String>) -> Self {
        Self {
            id: task.id.clone(),
            title: task.title.clone(),
            priority: task.priority.as_u8(),
            priority_label: priority_label(task.priority),
            status: task.status.as_str().to_string(),
            in_progress: task.in_progress,
            requested: task.requested,
            blocked_by,
        }
    }
}

/// Work item output with full details.
#[derive(Debug, Serialize)]
struct WorkItemOutput {
    id: String,
    title: String,
    description: String,
    priority: u8,
    priority_label: &'static str,
    status: String,
    in_progress: bool,
    requested: bool,
    created_at: String,
    updated_at: String,
    dependencies: Vec<String>,
    guidance: Vec<String>,
}

impl WorkItemOutput {
    fn from_task(task: &crate::tasks::Task, deps: Vec<String>, guidance: Vec<String>) -> Self {
        Self {
            id: task.id.clone(),
            title: task.title.clone(),
            description: task.description.clone(),
            priority: task.priority.as_u8(),
            priority_label: priority_label(task.priority),
            status: task.status.as_str().to_string(),
            in_progress: task.in_progress,
            requested: task.requested,
            created_at: task.created_at.clone(),
            updated_at: task.updated_at.clone(),
            dependencies: deps,
            guidance,
        }
    }
}

/// Full work item with notes and how-tos.
#[derive(Debug, Serialize)]
struct FullWorkItemOutput {
    #[serde(flatten)]
    task: WorkItemOutput,
    notes: Vec<NoteOutput>,
    howtos: Vec<HowToOutput>,
}

/// Work item suggestion from `next` command.
#[derive(Debug, Serialize)]
struct WorkItemSuggestion {
    #[serde(flatten)]
    task: WorkItemOutput,
    notes: Vec<NoteOutput>,
    howtos: Vec<HowToOutput>,
    message: String,
}

/// Note output.
#[derive(Debug, Serialize)]
struct NoteOutput {
    id: i64,
    content: String,
    created_at: String,
}

impl From<crate::tasks::Note> for NoteOutput {
    fn from(n: crate::tasks::Note) -> Self {
        Self { id: n.id, content: n.content, created_at: n.created_at }
    }
}

/// Note output with work item ID.
#[derive(Debug, Serialize)]
struct NoteWithWorkItemOutput {
    id: i64,
    work_item_id: String,
    content: String,
    created_at: String,
}

/// How-to output.
#[derive(Debug, Serialize)]
struct HowToOutput {
    id: String,
    title: String,
    instructions: String,
    created_at: String,
    updated_at: String,
}

impl From<&crate::tasks::HowTo> for HowToOutput {
    fn from(h: &crate::tasks::HowTo) -> Self {
        Self {
            id: h.id.clone(),
            title: h.title.clone(),
            instructions: h.instructions.clone(),
            created_at: h.created_at.clone(),
            updated_at: h.updated_at.clone(),
        }
    }
}

/// Question output.
#[derive(Debug, Serialize)]
struct QuestionOutput {
    id: String,
    text: String,
    answer: Option<String>,
    is_answered: bool,
    created_at: String,
    answered_at: Option<String>,
}

impl From<&crate::tasks::Question> for QuestionOutput {
    fn from(q: &crate::tasks::Question) -> Self {
        Self {
            id: q.id.clone(),
            text: q.text.clone(),
            answer: q.answer.clone(),
            is_answered: q.is_answered(),
            created_at: q.created_at.clone(),
            answered_at: q.answered_at.clone(),
        }
    }
}

/// Get the string label for a priority level.
const fn priority_label(priority: Priority) -> &'static str {
    match priority {
        Priority::Critical => "critical",
        Priority::High => "high",
        Priority::Medium => "medium",
        Priority::Low => "low",
        Priority::Backlog => "backlog",
    }
}
