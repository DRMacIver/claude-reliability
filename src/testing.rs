//! Testing utilities and mock implementations.
//!
//! These types are provided for use in tests. They may appear unused in
//! the library itself but are consumed by unit tests.

#![allow(dead_code)]
#![allow(clippy::needless_pass_by_ref_mut)] // &mut self for ergonomics with RefCell

use crate::error::Result;
use crate::traits::{CommandOutput, CommandRunner, SubAgent, SubAgentDecision};
use std::cell::RefCell;
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
    reflections: RefCell<Vec<(bool, String)>>,
    question_index: RefCell<usize>,
    review_index: RefCell<usize>,
    reflection_index: RefCell<usize>,
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

    /// Add an expected reflection result.
    pub fn expect_reflection(&mut self, complete: bool, feedback: &str) {
        self.reflections.borrow_mut().push((complete, feedback.to_string()));
    }
}

impl SubAgent for MockSubAgent {
    fn decide_on_question(
        &self,
        _assistant_output: &str,
        _user_recency_minutes: u32,
    ) -> Result<SubAgentDecision> {
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

    fn reflect_on_work(&self, _assistant_output: &str, _git_diff: &str) -> Result<(bool, String)> {
        let mut index = self.reflection_index.borrow_mut();
        let reflections = self.reflections.borrow();

        assert!(*index < reflections.len(), "No more reflections expected");

        let reflection = reflections[*index].clone();
        *index += 1;
        Ok(reflection)
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

    #[test]
    fn test_mock_sub_agent() {
        let mut agent = MockSubAgent::new();
        agent.expect_question_decision(SubAgentDecision::Continue);

        let decision = agent.decide_on_question("test", 5).unwrap();
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
    fn test_mock_sub_agent_reflect_on_work() {
        let mut agent = MockSubAgent::new();
        agent.expect_reflection(true, "Work looks complete");

        let (complete, feedback) = agent.reflect_on_work("output", "diff").unwrap();
        assert!(complete);
        assert_eq!(feedback, "Work looks complete");
    }

    #[test]
    fn test_mock_sub_agent_reflect_on_work_incomplete() {
        let mut agent = MockSubAgent::new();
        agent.expect_reflection(false, "Missing test coverage");

        let (complete, feedback) = agent.reflect_on_work("output", "diff").unwrap();
        assert!(!complete);
        assert_eq!(feedback, "Missing test coverage");
    }

    #[test]
    #[should_panic(expected = "No more reflections expected")]
    fn test_mock_sub_agent_reflect_on_work_no_expectation() {
        let agent = MockSubAgent::new();
        let _ = agent.reflect_on_work("output", "diff");
    }
}
