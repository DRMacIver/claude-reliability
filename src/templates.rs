//! Template loading and rendering using Tera.
//!
//! This module provides infrastructure for loading user-facing messages from
//! external template files, with embedded fallbacks for when files don't exist.

use crate::error::{Error, Result};
use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::path::Path;
use std::sync::RwLock;
use tera::{Context, Tera};

/// Default templates directory relative to crate root.
const TEMPLATES_DIR: &str = "templates";

/// Embedded default templates for fallback when files don't exist.
static EMBEDDED_TEMPLATES: Lazy<HashMap<&'static str, &'static str>> = Lazy::new(|| {
    let mut m = HashMap::new();

    // Prompts
    m.insert(
        "prompts/question_decision.tera",
        include_str!("../templates/prompts/question_decision.tera"),
    );
    m.insert("prompts/code_review.tera", include_str!("../templates/prompts/code_review.tera"));
    m.insert(
        "prompts/emergency_stop_decision.tera",
        include_str!("../templates/prompts/emergency_stop_decision.tera"),
    );
    m.insert(
        "prompts/create_question_decision.tera",
        include_str!("../templates/prompts/create_question_decision.tera"),
    );

    // Stop hook messages
    m.insert(
        "messages/stop/problem_mode_exit.tera",
        include_str!("../templates/messages/stop/problem_mode_exit.tera"),
    );
    m.insert(
        "messages/stop/problem_mode_activated.tera",
        include_str!("../templates/messages/stop/problem_mode_activated.tera"),
    );
    m.insert(
        "messages/stop/api_error_loop.tera",
        include_str!("../templates/messages/stop/api_error_loop.tera"),
    );
    m.insert(
        "messages/stop/validation_failed.tera",
        include_str!("../templates/messages/stop/validation_failed.tera"),
    );
    m.insert(
        "messages/stop/uncommitted_changes.tera",
        include_str!("../templates/messages/stop/uncommitted_changes.tera"),
    );
    m.insert(
        "messages/stop/unpushed_commits.tera",
        include_str!("../templates/messages/stop/unpushed_commits.tera"),
    );
    m.insert(
        "messages/stop/open_issues_remaining.tera",
        include_str!("../templates/messages/stop/open_issues_remaining.tera"),
    );
    m.insert(
        "messages/stop/staleness_detected.tera",
        include_str!("../templates/messages/stop/staleness_detected.tera"),
    );
    m.insert(
        "messages/stop/auto_work_tasks.tera",
        include_str!("../templates/messages/stop/auto_work_tasks.tera"),
    );
    m.insert(
        "messages/stop/work_item_reminder.tera",
        include_str!("../templates/messages/stop/work_item_reminder.tera"),
    );

    // Other hook messages
    m.insert(
        "messages/problem_mode_block.tera",
        include_str!("../templates/messages/problem_mode_block.tera"),
    );
    m.insert(
        "messages/protect_config_write.tera",
        include_str!("../templates/messages/protect_config_write.tera"),
    );
    m.insert(
        "messages/protect_config_delete.tera",
        include_str!("../templates/messages/protect_config_delete.tera"),
    );
    m.insert(
        "messages/no_verify_block.tera",
        include_str!("../templates/messages/no_verify_block.tera"),
    );
    m.insert(
        "messages/session_intro.tera",
        include_str!("../templates/messages/session_intro.tera"),
    );
    m.insert("messages/require_task.tera", include_str!("../templates/messages/require_task.tera"));
    m.insert(
        "messages/enter_plan_mode_intent.tera",
        include_str!("../templates/messages/enter_plan_mode_intent.tera"),
    );

    m
});

/// Global template engine with caching.
static TERA: Lazy<RwLock<Option<Tera>>> = Lazy::new(|| RwLock::new(None));

/// Initialize the template engine with templates from the specified directory.
///
/// If the directory doesn't exist, templates will be loaded from embedded defaults.
///
/// # Errors
///
/// Returns an error if the templates directory exists but contains invalid templates.
///
/// # Panics
///
/// Panics if an embedded template fails to add to the engine. This should never
/// happen as embedded templates are verified by `test_all_embedded_templates_render`.
pub fn init_templates(templates_dir: Option<&Path>) -> Result<()> {
    let dir = templates_dir.map_or_else(
        || std::env::current_dir().unwrap_or_default().join(TEMPLATES_DIR),
        Path::to_path_buf,
    );

    let mut tera = Tera::default();

    // Try to load from filesystem first
    if dir.exists() {
        let glob_pattern = format!("{}/**/*.tera", dir.display());
        match Tera::new(&glob_pattern) {
            Ok(t) => {
                tera = t;
            }
            Err(e) => {
                return Err(Error::Template(format!(
                    "Failed to load templates from {}: {e}",
                    dir.display()
                )));
            }
        }
    }

    // Add any missing templates from embedded defaults
    // These are verified by test_all_embedded_templates_render, so failure here
    // indicates a bug in the template that should have been caught in tests.
    for (name, content) in EMBEDDED_TEMPLATES.iter() {
        if tera.get_template(name).is_err() {
            tera.add_raw_template(name, content)
                .expect("embedded template should be valid - verified by tests");
        }
    }

    *TERA.write().map_err(|e| Error::Template(e.to_string()))? = Some(tera);

    Ok(())
}

/// Render a template with the given context.
///
/// Templates are lazy-loaded from the filesystem on first use, with embedded
/// defaults as fallback.
///
/// # Arguments
///
/// * `name` - Template name (e.g., `prompts/code_review.tera`)
/// * `context` - Tera context with variables for the template
///
/// # Errors
///
/// Returns an error if the template doesn't exist or rendering fails.
pub fn render(name: &str, context: &Context) -> Result<String> {
    // Ensure templates are initialized
    let needs_init = TERA.read().map_err(|e| Error::Template(e.to_string()))?.is_none();

    if needs_init {
        init_templates(None)?;
    }

    let guard = TERA.read().map_err(|e| Error::Template(e.to_string()))?;
    let tera = guard.as_ref().ok_or_else(|| Error::Template("Templates not initialized".into()))?;
    let rendered = tera
        .render(name, context)
        .map_err(|e| Error::Template(format!("Failed to render template {name}: {e}")))?;
    drop(guard);

    Ok(rendered)
}

/// Render a template with a simple key-value context.
///
/// Convenience wrapper around [`render`] for simple cases.
///
/// # Arguments
///
/// * `name` - Template name
/// * `vars` - Slice of (key, value) pairs for the context
///
/// # Errors
///
/// Returns an error if the template doesn't exist or rendering fails.
pub fn render_with_vars(name: &str, vars: &[(&str, &str)]) -> Result<String> {
    let mut context = Context::new();
    for (key, value) in vars {
        context.insert(*key, value);
    }
    render(name, &context)
}

/// Create a new Tera context.
///
/// Convenience function for creating contexts with mixed types.
#[must_use]
pub fn context() -> Context {
    Context::new()
}

/// Reset the template cache, forcing re-initialization on next use.
///
/// Useful for testing or when template files have been modified.
///
/// # Errors
///
/// Returns an error if the write lock cannot be acquired.
pub fn reset_cache() -> Result<()> {
    *TERA.write().map_err(|e| Error::Template(e.to_string()))? = None;
    Ok(())
}

/// Get the list of all embedded template names.
///
/// Useful for testing that all templates can be rendered.
#[must_use]
pub fn embedded_template_names() -> Vec<&'static str> {
    EMBEDDED_TEMPLATES.keys().copied().collect()
}

/// Verify all embedded templates can be rendered with sample data.
///
/// This function creates sample contexts for each template and verifies
/// they render without errors.
///
/// # Errors
///
/// Returns an error if any template fails to render.
pub fn verify_all_templates() -> Result<()> {
    reset_cache()?;
    init_templates(Some(Path::new("/nonexistent")))?;

    for name in embedded_template_names() {
        let ctx = sample_context_for(name);
        render(name, &ctx)
            .map_err(|e| Error::Template(format!("Template {name} failed to render: {e}")))?;
    }

    Ok(())
}

/// Create a sample context with all variables a template might need.
fn sample_context_for(template_name: &str) -> Context {
    let mut ctx = Context::new();

    // Add all possible variables with sample values
    // Prompts
    ctx.insert("assistant_output", "Sample assistant output");
    ctx.insert("user_recency_minutes", &5_u32);
    ctx.insert("guide_section", "Sample review guidelines");
    ctx.insert("files_list", "- file1.rs\n- file2.rs");
    ctx.insert("diff", "+sample diff content");

    // Stop messages
    ctx.insert("error_count", &3_u32);
    ctx.insert("check_cmd", "just check");
    ctx.insert("stdout", "sample stdout");
    ctx.insert("stderr", "sample stderr");
    ctx.insert("changes_description", "1 modified file");
    ctx.insert("quality_failed", &false);
    ctx.insert("quality_output", "");
    ctx.insert("quality_check_enabled", &true);
    ctx.insert("require_push", &true);
    ctx.insert(
        "human_input_phrase",
        "I have completed all work that I can and require human input to proceed.",
    );
    ctx.insert("commits_ahead", &2_u32);
    ctx.insert("open_count", &3_u32);
    ctx.insert("change_type", "git");
    ctx.insert("iterations_since_change", &2_u32);
    ctx.insert("iteration", &5_u32);
    ctx.insert("staleness_threshold", &5_u32);
    ctx.insert("task_count", &3_u32);
    ctx.insert("idle_minutes", &30_u32);

    // Emergency stop
    ctx.insert("explanation", "I cannot proceed because the API key is missing.");

    // Create question
    ctx.insert("question_text", "Should I continue with the remaining tasks?");

    // Other hooks
    ctx.insert("tool_name", "Bash");
    ctx.insert("session_notes_path", ".claude/jkw-session.local.md");
    ctx.insert("config_path", ".claude/reliability-config.yaml");
    ctx.insert("acknowledgment", "I promise the user has said I can use --no-verify here");

    // For uncommitted_changes template - lists
    if template_name.contains("uncommitted_changes") {
        ctx.insert("suppression_violations", &Vec::<String>::new());
        ctx.insert("empty_except_violations", &Vec::<String>::new());
        ctx.insert("secret_violations", &Vec::<String>::new());
        ctx.insert("todo_warnings", &Vec::<String>::new());
        ctx.insert("untracked_files", &Vec::<String>::new());
    }

    ctx
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    #[serial_test::serial]
    fn test_init_with_no_templates_dir() {
        reset_cache().unwrap();
        init_templates(Some(Path::new("/nonexistent"))).unwrap();

        // Should still work with embedded templates
        let mut ctx = Context::new();
        ctx.insert("tool_name", "Test");
        let result = render("messages/problem_mode_block.tera", &ctx).unwrap();
        assert!(result.contains("Problem Mode"));
    }

    #[test]
    #[serial_test::serial]
    fn test_render_with_vars() {
        reset_cache().unwrap();
        init_templates(Some(Path::new("/nonexistent"))).unwrap();

        let result =
            render_with_vars("messages/problem_mode_block.tera", &[("tool_name", "Bash")]).unwrap();
        assert!(result.contains("Bash"));
    }

    #[test]
    #[serial_test::serial]
    fn test_filesystem_templates_override_embedded() {
        reset_cache().unwrap();

        let dir = TempDir::new().unwrap();
        let template_dir = dir.path().join("messages");
        fs::create_dir_all(&template_dir).unwrap();
        fs::write(template_dir.join("problem_mode_block.tera"), "CUSTOM: {{ tool_name }}").unwrap();

        init_templates(Some(dir.path())).unwrap();

        let result =
            render_with_vars("messages/problem_mode_block.tera", &[("tool_name", "Custom")])
                .unwrap();
        assert_eq!(result, "CUSTOM: Custom");
    }

    #[test]
    fn test_context_helper() {
        let mut ctx = context();
        ctx.insert("key", "value");
        // Just verify it compiles and works
        assert!(ctx.contains_key("key"));
    }

    #[test]
    #[serial_test::serial]
    fn test_render_missing_template_fails() {
        reset_cache().unwrap();
        init_templates(Some(Path::new("/nonexistent"))).unwrap();

        let result = render("nonexistent/template.tera", &Context::new());
        assert!(result.is_err());
    }

    #[test]
    #[serial_test::serial]
    fn test_lazy_init() {
        reset_cache().unwrap();
        // render() should auto-initialize
        let mut ctx = Context::new();
        ctx.insert("tool_name", "Test");
        let result = render("messages/problem_mode_block.tera", &ctx).unwrap();
        assert!(result.contains("Problem Mode"));
    }

    #[test]
    #[serial_test::serial]
    fn test_all_embedded_templates_render() {
        // This test verifies that ALL embedded templates can be rendered
        // with sample data. This catches missing variables and syntax errors.
        verify_all_templates().unwrap();
    }

    #[test]
    fn test_embedded_template_count() {
        // Ensure we have all expected templates
        let names = embedded_template_names();
        assert!(names.len() >= 16, "Expected at least 16 templates, got {}", names.len());
    }

    #[test]
    #[serial_test::serial]
    fn test_init_with_invalid_templates_fails() {
        reset_cache().unwrap();

        let dir = TempDir::new().unwrap();
        let template_dir = dir.path().join("messages");
        fs::create_dir_all(&template_dir).unwrap();

        // Create an invalid template with syntax error (unclosed tag)
        fs::write(template_dir.join("invalid.tera"), "{% if foo %}unclosed if tag without endif")
            .unwrap();

        // Should fail because of the invalid template
        let result = init_templates(Some(dir.path()));
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Failed to load templates"), "Error was: {err}");
    }
}
