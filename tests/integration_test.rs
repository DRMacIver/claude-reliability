//! Integration tests for `claude_reliability`.

use claude_reliability::VERSION;

#[test]
fn test_version_exists() {
    assert!(!VERSION.is_empty());
}

#[test]
fn test_real_command_runner() {
    use claude_reliability::CommandRunner;
    use claude_reliability::RealCommandRunner;

    let runner = RealCommandRunner::new();
    let output = runner.run("echo", &["hello"], None).unwrap();
    assert!(output.success());
    assert!(output.stdout.contains("hello"));
}
