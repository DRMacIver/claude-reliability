//! Integration tests for claude_reliability.

use claude_reliability::greet;

#[test]
fn integration_test_greet() {
    let result = greet("Integration");
    assert!(result.contains("Integration"));
}
