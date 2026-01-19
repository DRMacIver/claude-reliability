//! Integration tests for `claude_reliability`.

use claude_reliability::VERSION;

#[test]
fn integration_test_version() {
    assert!(!VERSION.is_empty());
}
