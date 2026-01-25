//! Testing utilities and mock implementations.
//!
//! These types are provided for use in tests. They may appear unused in
//! the library itself but are consumed by unit tests.

#![allow(dead_code)]
#![allow(clippy::needless_pass_by_ref_mut)] // &mut self for ergonomics with RefCell

use crate::error::Result;
use crate::traits::{
    CommandOutput, CommandRunner, QuestionContext, StateStore, SubAgent, SubAgentDecision,
};
use std::cell::RefCell;
use std::collections::HashSet;
use std::time::Duration;

/// A mock command runner for testing.
///
/// Records expected commands and their outputs, then verifies they were called.
#[derive(Debug, Default)]
pub struct MockCommandRunner {
    expectations: RefCell<Vec<(String, Vec<String>, CommandOutput)>>,
    available_programs: RefCell<Vec<String>>,
    call_index: RefCell<usize>,
}

impl MockCommandRunner {
    /// Create a new mock command runner.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add an expected command and its output.
    pub fn expect(&mut self, program: &str, args: &[&str], output: CommandOutput) {
        self.expectations.borrow_mut().push((
            program.to_string(),
            args.iter().map(|s| (*s).to_string()).collect(),
            output,
        ));
    }

    /// Add a program as available.
    pub fn set_available(&mut self, program: &str) {
        self.available_programs.borrow_mut().push(program.to_string());
    }

    /// Verify all expected commands were called.
    ///
    /// # Panics
    ///
    /// Panics if not all expected commands were called.
    pub fn verify(&self) {
        let index = *self.call_index.borrow();
        let expected = self.expectations.borrow().len();
        assert_eq!(
            index, expected,
            "Expected {expected} command calls, but only {index} were made"
        );
    }
}

impl CommandRunner for MockCommandRunner {
    fn run(
        &self,
        program: &str,
        args: &[&str],
        _timeout: Option<Duration>,
    ) -> Result<CommandOutput> {
        let mut index = self.call_index.borrow_mut();
        let expectations = self.expectations.borrow();

        assert!(
            *index < expectations.len(),
            "Unexpected command call: {program} {args:?} (no more expectations)"
        );

        let (exp_program, exp_args, output) = &expectations[*index];
        let args_vec: Vec<String> = args.iter().map(|s| (*s).to_string()).collect();

        assert!(
            !(program != exp_program || &args_vec != exp_args),
            "Command mismatch at index {}:\n  Expected: {} {:?}\n  Got: {} {:?}",
            *index,
            exp_program,
            exp_args,
            program,
            args
        );

        *index += 1;
        Ok(output.clone())
    }

    fn is_available(&self, program: &str) -> bool {
        self.available_programs.borrow().contains(&program.to_string())
    }
}

/// A mock sub-agent for testing.
#[derive(Debug, Default)]
pub struct MockSubAgent {
    question_decisions: RefCell<Vec<SubAgentDecision>>,
    code_reviews: RefCell<Vec<(bool, String)>>,
    question_index: RefCell<usize>,
    review_index: RefCell<usize>,
}

impl MockSubAgent {
    /// Create a new mock sub-agent.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add an expected decision for a question.
    pub fn expect_question_decision(&mut self, decision: SubAgentDecision) {
        self.question_decisions.borrow_mut().push(decision);
    }

    /// Add an expected code review result.
    pub fn expect_review(&mut self, approved: bool, feedback: &str) {
        self.code_reviews.borrow_mut().push((approved, feedback.to_string()));
    }
}

impl SubAgent for MockSubAgent {
    fn decide_on_question(&self, _context: &QuestionContext) -> Result<SubAgentDecision> {
        let mut index = self.question_index.borrow_mut();
        let decisions = self.question_decisions.borrow();

        assert!(*index < decisions.len(), "No more question decisions expected");

        let decision = decisions[*index].clone();
        *index += 1;
        Ok(decision)
    }

    fn review_code(
        &self,
        _diff: &str,
        _files: &[String],
        _review_guide: Option<&str>,
    ) -> Result<(bool, String)> {
        let mut index = self.review_index.borrow_mut();
        let reviews = self.code_reviews.borrow();

        assert!(*index < reviews.len(), "No more code reviews expected");

        let review = reviews[*index].clone();
        *index += 1;
        Ok(review)
    }
}

/// A command runner that always fails, for testing error paths.
#[derive(Debug, Default)]
pub struct FailingCommandRunner {
    error_message: String,
}

impl FailingCommandRunner {
    /// Create a new failing command runner with the specified error message.
    #[must_use]
    pub fn new(error_message: impl Into<String>) -> Self {
        Self { error_message: error_message.into() }
    }
}

impl CommandRunner for FailingCommandRunner {
    fn run(
        &self,
        _program: &str,
        _args: &[&str],
        _timeout: Option<Duration>,
    ) -> Result<CommandOutput> {
        Err(std::io::Error::other(self.error_message.clone()).into())
    }

    fn is_available(&self, _program: &str) -> bool {
        false
    }
}

/// A sub-agent that always fails, for testing error paths.
#[derive(Debug, Default)]
pub struct FailingSubAgent {
    error_message: String,
}

impl FailingSubAgent {
    /// Create a new failing sub-agent with the specified error message.
    #[must_use]
    pub fn new(error_message: impl Into<String>) -> Self {
        Self { error_message: error_message.into() }
    }
}

impl SubAgent for FailingSubAgent {
    fn decide_on_question(&self, _context: &QuestionContext) -> Result<SubAgentDecision> {
        Err(std::io::Error::other(self.error_message.clone()).into())
    }

    fn review_code(
        &self,
        _diff: &str,
        _files: &[String],
        _review_guide: Option<&str>,
    ) -> Result<(bool, String)> {
        Err(std::io::Error::other(self.error_message.clone()).into())
    }
}

/// A mock state store for testing.
///
/// Stores state in memory and records operations for verification.
#[derive(Debug, Default)]
pub struct MockStateStore {
    markers: RefCell<HashSet<String>>,
}

impl MockStateStore {
    /// Create a new mock state store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Get all currently set markers.
    pub fn get_markers(&self) -> HashSet<String> {
        self.markers.borrow().clone()
    }
}

impl StateStore for MockStateStore {
    fn has_marker(&self, name: &str) -> bool {
        self.markers.borrow().contains(name)
    }

    fn set_marker(&self, name: &str) -> Result<()> {
        self.markers.borrow_mut().insert(name.to_string());
        Ok(())
    }

    fn clear_marker(&self, name: &str) -> Result<()> {
        self.markers.borrow_mut().remove(name);
        Ok(())
    }
}

/// A state store that always fails, for testing error paths.
#[derive(Debug, Default)]
pub struct FailingStateStore {
    error_message: String,
}

impl FailingStateStore {
    /// Create a new failing state store with the specified error message.
    #[must_use]
    pub fn new(error_message: impl Into<String>) -> Self {
        Self { error_message: error_message.into() }
    }
}

impl StateStore for FailingStateStore {
    fn has_marker(&self, _name: &str) -> bool {
        false
    }

    fn set_marker(&self, _name: &str) -> Result<()> {
        Err(std::io::Error::other(self.error_message.clone()).into())
    }

    fn clear_marker(&self, _name: &str) -> Result<()> {
        Err(std::io::Error::other(self.error_message.clone()).into())
    }
}

/// A state store that wraps `MockStateStore` but fails on `clear_marker`.
///
/// This is useful for testing error handling when marker clearing fails.
#[derive(Debug)]
pub struct FailingClearMarkerStore {
    inner: MockStateStore,
    error_message: String,
}

impl FailingClearMarkerStore {
    /// Create a new failing clear marker store wrapping the given mock.
    #[must_use]
    pub fn new(inner: MockStateStore, error_message: impl Into<String>) -> Self {
        Self { inner, error_message: error_message.into() }
    }
}

/// A state store that wraps `MockStateStore` but fails on `set_marker`.
///
/// This is useful for testing error handling when marker setting fails.
#[derive(Debug)]
pub struct FailingSetMarkerStore {
    inner: MockStateStore,
    error_message: String,
}

impl FailingSetMarkerStore {
    /// Create a new failing set marker store wrapping the given mock.
    #[must_use]
    pub fn new(inner: MockStateStore, error_message: impl Into<String>) -> Self {
        Self { inner, error_message: error_message.into() }
    }
}

impl StateStore for FailingClearMarkerStore {
    fn has_marker(&self, name: &str) -> bool {
        self.inner.has_marker(name)
    }

    fn set_marker(&self, name: &str) -> Result<()> {
        self.inner.set_marker(name)
    }

    fn clear_marker(&self, _name: &str) -> Result<()> {
        Err(std::io::Error::other(self.error_message.clone()).into())
    }
}

impl StateStore for FailingSetMarkerStore {
    fn has_marker(&self, name: &str) -> bool {
        self.inner.has_marker(name)
    }

    fn set_marker(&self, _name: &str) -> Result<()> {
        Err(std::io::Error::other(self.error_message.clone()).into())
    }

    fn clear_marker(&self, name: &str) -> Result<()> {
        self.inner.clear_marker(name)
    }
}

/// A mock task store where all write operations fail with a configurable error.
///
/// Used to test error handling paths in code that creates or modifies tasks.
#[cfg(test)]
pub struct FailingTaskStore {
    error_message: String,
}

#[cfg(test)]
impl FailingTaskStore {
    /// Create a new failing task store with the specified error message.
    pub fn new(error_message: impl Into<String>) -> Self {
        Self { error_message: error_message.into() }
    }
}

#[cfg(test)]
impl crate::tasks::TaskStore for FailingTaskStore {
    fn create_task(
        &self,
        _title: &str,
        _description: &str,
        _priority: crate::tasks::Priority,
    ) -> Result<crate::tasks::Task> {
        Err(crate::error::Error::Config(self.error_message.clone()))
    }

    fn get_task(&self, _id: &str) -> Result<Option<crate::tasks::Task>> {
        Ok(None)
    }

    fn update_task(
        &self,
        _id: &str,
        _update: crate::tasks::TaskUpdate,
    ) -> Result<Option<crate::tasks::Task>> {
        Ok(None)
    }

    fn delete_task(&self, _id: &str) -> Result<bool> {
        Ok(false)
    }

    fn list_tasks(&self, _filter: crate::tasks::TaskFilter) -> Result<Vec<crate::tasks::Task>> {
        Ok(vec![])
    }

    fn add_dependency(&self, _task_id: &str, _depends_on: &str) -> Result<()> {
        Ok(())
    }

    fn remove_dependency(&self, _task_id: &str, _depends_on: &str) -> Result<bool> {
        Ok(false)
    }

    fn get_dependencies(&self, _task_id: &str) -> Result<Vec<String>> {
        Ok(vec![])
    }

    fn get_dependents(&self, _task_id: &str) -> Result<Vec<String>> {
        Ok(vec![])
    }

    fn add_note(&self, _task_id: &str, _content: &str) -> Result<crate::tasks::Note> {
        Err(crate::error::Error::Config(self.error_message.clone()))
    }

    fn get_notes(&self, _task_id: &str) -> Result<Vec<crate::tasks::Note>> {
        Ok(vec![])
    }

    fn delete_note(&self, _note_id: i64) -> Result<bool> {
        Ok(false)
    }

    fn search_tasks(&self, _query: &str) -> Result<Vec<crate::tasks::Task>> {
        Ok(vec![])
    }

    fn get_audit_log(
        &self,
        _task_id: Option<&str>,
        _limit: Option<usize>,
    ) -> Result<Vec<crate::tasks::AuditEntry>> {
        Ok(vec![])
    }

    fn get_ready_tasks(&self) -> Result<Vec<crate::tasks::Task>> {
        Ok(vec![])
    }

    fn pick_task(&self) -> Result<Option<crate::tasks::Task>> {
        Ok(None)
    }

    fn create_howto(&self, _title: &str, _instructions: &str) -> Result<crate::tasks::HowTo> {
        Err(crate::error::Error::Config(self.error_message.clone()))
    }

    fn get_howto(&self, _id: &str) -> Result<Option<crate::tasks::HowTo>> {
        Ok(None)
    }

    fn update_howto(
        &self,
        _id: &str,
        _update: crate::tasks::HowToUpdate,
    ) -> Result<Option<crate::tasks::HowTo>> {
        Ok(None)
    }

    fn delete_howto(&self, _id: &str) -> Result<bool> {
        Ok(false)
    }

    fn list_howtos(&self) -> Result<Vec<crate::tasks::HowTo>> {
        Ok(vec![])
    }

    fn search_howtos(&self, _query: &str) -> Result<Vec<crate::tasks::HowTo>> {
        Ok(vec![])
    }

    fn link_task_to_howto(&self, _task_id: &str, _howto_id: &str) -> Result<()> {
        Ok(())
    }

    fn unlink_task_from_howto(&self, _task_id: &str, _howto_id: &str) -> Result<bool> {
        Ok(false)
    }

    fn get_task_guidance(&self, _task_id: &str) -> Result<Vec<String>> {
        Ok(vec![])
    }

    fn create_question(&self, _text: &str) -> Result<crate::tasks::Question> {
        Err(crate::error::Error::Config(self.error_message.clone()))
    }

    fn get_question(&self, _id: &str) -> Result<Option<crate::tasks::Question>> {
        Ok(None)
    }

    fn answer_question(&self, _id: &str, _answer: &str) -> Result<Option<crate::tasks::Question>> {
        Ok(None)
    }

    fn delete_question(&self, _id: &str) -> Result<bool> {
        Ok(false)
    }

    fn list_questions(&self, _unanswered_only: bool) -> Result<Vec<crate::tasks::Question>> {
        Ok(vec![])
    }

    fn search_questions(&self, _query: &str) -> Result<Vec<crate::tasks::Question>> {
        Ok(vec![])
    }

    fn link_task_to_question(&self, _task_id: &str, _question_id: &str) -> Result<()> {
        Ok(())
    }

    fn unlink_task_from_question(&self, _task_id: &str, _question_id: &str) -> Result<bool> {
        Ok(false)
    }

    fn get_task_questions(&self, _task_id: &str) -> Result<Vec<String>> {
        Ok(vec![])
    }

    fn get_blocking_questions(&self, _task_id: &str) -> Result<Vec<crate::tasks::Question>> {
        Ok(vec![])
    }

    fn get_question_blocked_tasks(&self) -> Result<Vec<crate::tasks::Task>> {
        Ok(vec![])
    }

    fn has_in_progress_task(&self) -> Result<bool> {
        Ok(false)
    }

    fn get_in_progress_tasks(&self) -> Result<Vec<crate::tasks::Task>> {
        Ok(vec![])
    }

    fn request_tasks(&self, _task_ids: &[&str]) -> Result<usize> {
        Err(crate::error::Error::Config(self.error_message.clone()))
    }

    fn request_all_open(&self) -> Result<usize> {
        Ok(0)
    }

    fn is_request_mode_active(&self) -> Result<bool> {
        Ok(false)
    }

    fn clear_request_mode(&self) -> Result<()> {
        Ok(())
    }

    fn get_incomplete_requested_tasks(&self) -> Result<Vec<crate::tasks::Task>> {
        Ok(vec![])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mock_command_runner() {
        let mut runner = MockCommandRunner::new();
        runner.expect(
            "echo",
            &["hello"],
            CommandOutput { exit_code: 0, stdout: "hello\n".to_string(), stderr: String::new() },
        );

        let output = runner.run("echo", &["hello"], None).unwrap();
        assert_eq!(output.stdout, "hello\n");
        runner.verify();
    }

    #[test]
    fn test_mock_command_runner_availability() {
        let mut runner = MockCommandRunner::new();
        runner.set_available("git");

        assert!(runner.is_available("git"));
        assert!(!runner.is_available("nonexistent"));
    }

    #[test]
    #[should_panic(expected = "Command mismatch")]
    fn test_mock_command_runner_wrong_command() {
        let mut runner = MockCommandRunner::new();
        runner.expect("echo", &["hello"], CommandOutput::default());

        let _ = runner.run("echo", &["world"], None);
    }

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
    fn test_mock_sub_agent() {
        let mut agent = MockSubAgent::new();
        agent.expect_question_decision(SubAgentDecision::Continue);

        let decision = agent.decide_on_question(&test_context("test")).unwrap();
        assert_eq!(decision, SubAgentDecision::Continue);
    }

    #[test]
    #[should_panic(expected = "no more expectations")]
    fn test_mock_command_runner_too_many_calls() {
        let runner = MockCommandRunner::new();
        // No expectations set, so any call should panic
        let _ = runner.run("echo", &["hello"], None);
    }

    #[test]
    #[should_panic(expected = "Expected 1 command calls")]
    fn test_mock_command_runner_verify_fails() {
        let mut runner = MockCommandRunner::new();
        runner.expect("echo", &["hello"], CommandOutput::default());
        // Don't make the call, so verify should fail
        runner.verify();
    }

    #[test]
    fn test_failing_command_runner() {
        let runner = FailingCommandRunner::new("test error");
        let result = runner.run("any", &["args"], None);
        assert!(result.is_err());
        assert!(!runner.is_available("any"));
    }

    #[test]
    fn test_failing_sub_agent() {
        let agent = FailingSubAgent::new("test error");

        // All methods should return errors
        assert!(agent.decide_on_question(&test_context("test")).is_err());
        assert!(agent.review_code("diff", &[], None).is_err());
    }

    #[test]
    fn test_mock_state_store_markers() {
        let store = MockStateStore::new();

        // Initially no markers
        assert!(!store.has_marker("test"));
        assert!(store.get_markers().is_empty());

        // Set marker
        store.set_marker("test").unwrap();
        assert!(store.has_marker("test"));
        assert!(store.get_markers().contains("test"));

        // Clear marker
        store.clear_marker("test").unwrap();
        assert!(!store.has_marker("test"));
    }

    #[test]
    fn test_failing_state_store() {
        let store = FailingStateStore::new("test error");

        // All methods should return errors or false
        assert!(!store.has_marker("test"));
        assert!(store.set_marker("test").is_err());
        assert!(store.clear_marker("test").is_err());
    }

    #[test]
    fn test_failing_clear_marker_store_delegates_other_methods() {
        let inner = MockStateStore::new();
        let store = FailingClearMarkerStore::new(inner, "clear failed");

        // Marker methods: has_marker and set_marker should delegate, clear_marker should fail
        assert!(!store.has_marker("test"));
        store.set_marker("test").unwrap();
        assert!(store.has_marker("test"));
        assert!(store.clear_marker("test").is_err()); // This should fail
    }

    #[test]
    fn test_failing_set_marker_store_delegates_other_methods() {
        let inner = MockStateStore::new();
        let store = FailingSetMarkerStore::new(inner, "set failed");

        // Marker methods: has_marker and clear_marker should delegate, set_marker should fail
        assert!(!store.has_marker("test"));
        assert!(store.set_marker("test").is_err());
        store.clear_marker("test").unwrap();
    }

    #[test]
    fn test_failing_task_store() {
        use crate::tasks::{Priority, TaskStore};

        let store = FailingTaskStore::new("test error");

        // Write operations should fail
        assert!(store.create_task("title", "desc", Priority::Medium).is_err());
        assert!(store.add_note("id", "note").is_err());
        assert!(store.create_howto("title", "instructions").is_err());
        assert!(store.create_question("text").is_err());
        assert!(store.request_tasks(&["id"]).is_err());

        // Read operations should return empty/None/false
        assert!(store.get_task("id").unwrap().is_none());
        assert!(store.update_task("id", crate::tasks::TaskUpdate::default()).unwrap().is_none());
        assert!(!store.delete_task("id").unwrap());
        assert!(store.list_tasks(crate::tasks::TaskFilter::default()).unwrap().is_empty());
        assert!(store.get_ready_tasks().unwrap().is_empty());
        assert!(store.pick_task().unwrap().is_none());
        assert!(!store.has_in_progress_task().unwrap());
        assert!(store.get_in_progress_tasks().unwrap().is_empty());

        // Dependencies
        store.add_dependency("a", "b").unwrap();
        assert!(!store.remove_dependency("a", "b").unwrap());
        assert!(store.get_dependencies("a").unwrap().is_empty());
        assert!(store.get_dependents("a").unwrap().is_empty());

        // Notes
        assert!(store.get_notes("id").unwrap().is_empty());
        assert!(!store.delete_note(1).unwrap());

        // Search & audit
        assert!(store.search_tasks("q").unwrap().is_empty());
        assert!(store.get_audit_log(None, None).unwrap().is_empty());

        // How-tos
        assert!(store.get_howto("id").unwrap().is_none());
        assert!(store.update_howto("id", crate::tasks::HowToUpdate::default()).unwrap().is_none());
        assert!(!store.delete_howto("id").unwrap());
        assert!(store.list_howtos().unwrap().is_empty());
        assert!(store.search_howtos("q").unwrap().is_empty());
        store.link_task_to_howto("a", "b").unwrap();
        assert!(!store.unlink_task_from_howto("a", "b").unwrap());
        assert!(store.get_task_guidance("id").unwrap().is_empty());

        // Questions
        assert!(store.get_question("id").unwrap().is_none());
        assert!(store.answer_question("id", "a").unwrap().is_none());
        assert!(!store.delete_question("id").unwrap());
        assert!(store.list_questions(false).unwrap().is_empty());
        assert!(store.search_questions("q").unwrap().is_empty());
        store.link_task_to_question("a", "b").unwrap();
        assert!(!store.unlink_task_from_question("a", "b").unwrap());
        assert!(store.get_task_questions("id").unwrap().is_empty());
        assert!(store.get_blocking_questions("id").unwrap().is_empty());
        assert!(store.get_question_blocked_tasks().unwrap().is_empty());

        // Request mode
        assert_eq!(store.request_all_open().unwrap(), 0);
        assert!(!store.is_request_mode_active().unwrap());
        store.clear_request_mode().unwrap();
        assert!(store.get_incomplete_requested_tasks().unwrap().is_empty());
    }
}
