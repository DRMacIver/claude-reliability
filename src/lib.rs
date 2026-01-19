//! claude_reliability library.
//!
//! This is the main library crate.

/// Library version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Greet a person by name.
///
/// # Examples
///
/// ```
/// use claude_reliability::greet;
/// assert_eq!(greet("World"), "Hello, World!");
/// ```
pub fn greet(name: &str) -> String {
    format!("Hello, {name}!")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_greet() {
        assert_eq!(greet("Rust"), "Hello, Rust!");
    }

    #[test]
    fn test_version_exists() {
        assert!(!VERSION.is_empty());
    }
}
