//! Testing utilities and mock implementations.
//!
//! These types are provided for use in tests. They may appear unused in
//! the library itself but are consumed by unit tests.

#![allow(dead_code)]
#![allow(clippy::needless_pass_by_ref_mut)] // &mut self for ergonomics with RefCell

use crate::error::Result;
use crate::session::SessionConfig;
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
    session_state: RefCell<Option<SessionConfig>>,
    issue_snapshot: RefCell<HashSet<String>>,
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
    fn get_session_state(&self) -> Result<Option<SessionConfig>> {
        Ok(self.session_state.borrow().clone())
    }

    fn set_session_state(&self, state: &SessionConfig) -> Result<()> {
        *self.session_state.borrow_mut() = Some(state.clone());
        Ok(())
    }

    fn clear_session_state(&self) -> Result<()> {
        *self.session_state.borrow_mut() = None;
        self.issue_snapshot.borrow_mut().clear();
        Ok(())
    }

    fn get_issue_snapshot(&self) -> Result<HashSet<String>> {
        Ok(self.issue_snapshot.borrow().clone())
    }

    fn set_issue_snapshot(&self, issues: &[String]) -> Result<()> {
        let mut snapshot = self.issue_snapshot.borrow_mut();
        snapshot.clear();
        snapshot.extend(issues.iter().cloned());
        Ok(())
    }

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
    fn get_session_state(&self) -> Result<Option<SessionConfig>> {
        Err(std::io::Error::other(self.error_message.clone()).into())
    }

    fn set_session_state(&self, _state: &SessionConfig) -> Result<()> {
        Err(std::io::Error::other(self.error_message.clone()).into())
    }

    fn clear_session_state(&self) -> Result<()> {
        Err(std::io::Error::other(self.error_message.clone()).into())
    }

    fn get_issue_snapshot(&self) -> Result<HashSet<String>> {
        Err(std::io::Error::other(self.error_message.clone()).into())
    }

    fn set_issue_snapshot(&self, _issues: &[String]) -> Result<()> {
        Err(std::io::Error::other(self.error_message.clone()).into())
    }

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
    fn get_session_state(&self) -> Result<Option<SessionConfig>> {
        self.inner.get_session_state()
    }

    fn set_session_state(&self, state: &SessionConfig) -> Result<()> {
        self.inner.set_session_state(state)
    }

    fn clear_session_state(&self) -> Result<()> {
        self.inner.clear_session_state()
    }

    fn get_issue_snapshot(&self) -> Result<HashSet<String>> {
        self.inner.get_issue_snapshot()
    }

    fn set_issue_snapshot(&self, issues: &[String]) -> Result<()> {
        self.inner.set_issue_snapshot(issues)
    }

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
    fn get_session_state(&self) -> Result<Option<SessionConfig>> {
        self.inner.get_session_state()
    }

    fn set_session_state(&self, state: &SessionConfig) -> Result<()> {
        self.inner.set_session_state(state)
    }

    fn clear_session_state(&self) -> Result<()> {
        self.inner.clear_session_state()
    }

    fn get_issue_snapshot(&self) -> Result<HashSet<String>> {
        self.inner.get_issue_snapshot()
    }

    fn set_issue_snapshot(&self, issues: &[String]) -> Result<()> {
        self.inner.set_issue_snapshot(issues)
    }

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
    fn test_mock_state_store_session_state() {
        let store = MockStateStore::new();

        // Initially no state
        assert!(store.get_session_state().unwrap().is_none());

        // Set state
        let state = SessionConfig {
            iteration: 5,
            last_issue_change_iteration: 3,
            issue_snapshot: vec!["a".to_string()],
            git_diff_hash: Some("hash".to_string()),
        };
        store.set_session_state(&state).unwrap();

        // Read back
        let read = store.get_session_state().unwrap().unwrap();
        assert_eq!(read.iteration, 5);
        assert_eq!(read.issue_snapshot, vec!["a"]);

        // Clear
        store.clear_session_state().unwrap();
        assert!(store.get_session_state().unwrap().is_none());
    }

    #[test]
    fn test_mock_state_store_issue_snapshot() {
        let store = MockStateStore::new();

        // Initially empty
        assert!(store.get_issue_snapshot().unwrap().is_empty());

        // Set snapshot
        store.set_issue_snapshot(&["a".to_string(), "b".to_string()]).unwrap();

        let snapshot = store.get_issue_snapshot().unwrap();
        assert_eq!(snapshot.len(), 2);
        assert!(snapshot.contains("a"));
        assert!(snapshot.contains("b"));
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
        assert!(store.get_session_state().is_err());
        assert!(store.set_session_state(&SessionConfig::default()).is_err());
        assert!(store.clear_session_state().is_err());
        assert!(store.get_issue_snapshot().is_err());
        assert!(store.set_issue_snapshot(&[]).is_err());
        assert!(!store.has_marker("test"));
        assert!(store.set_marker("test").is_err());
        assert!(store.clear_marker("test").is_err());
    }

    #[test]
    fn test_failing_clear_marker_store_delegates_other_methods() {
        let inner = MockStateStore::new();
        let store = FailingClearMarkerStore::new(inner, "clear failed");

        // Session state methods should delegate to inner
        assert!(store.get_session_state().unwrap().is_none());
        store.set_session_state(&SessionConfig::default()).unwrap();
        assert!(store.get_session_state().unwrap().is_some());
        store.clear_session_state().unwrap();
        assert!(store.get_session_state().unwrap().is_none());

        // Issue snapshot methods should delegate
        assert!(store.get_issue_snapshot().unwrap().is_empty());
        store.set_issue_snapshot(&["issue1".to_string()]).unwrap();
        assert!(store.get_issue_snapshot().unwrap().contains("issue1"));

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

        // Session state methods should delegate to inner
        assert!(store.get_session_state().unwrap().is_none());
        store.set_session_state(&SessionConfig::default()).unwrap();
        assert!(store.get_session_state().unwrap().is_some());
        store.clear_session_state().unwrap();
        assert!(store.get_session_state().unwrap().is_none());

        // Issue snapshot methods should delegate
        assert!(store.get_issue_snapshot().unwrap().is_empty());
        store.set_issue_snapshot(&["issue1".to_string()]).unwrap();
        assert!(store.get_issue_snapshot().unwrap().contains("issue1"));

        // Marker methods: has_marker and clear_marker should delegate, set_marker should fail
        assert!(!store.has_marker("test"));
        assert!(store.set_marker("test").is_err());
        store.clear_marker("test").unwrap();
    }
}
