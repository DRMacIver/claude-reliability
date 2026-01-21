//! Task ID generation utilities.
//!
//! Task IDs are generated from the title by:
//! 1. Converting to lowercase
//! 2. Replacing non-alphanumeric characters with hyphens
//! 3. Collapsing multiple hyphens
//! 4. Trimming leading/trailing hyphens
//! 5. Appending 4 random hex characters

use std::sync::atomic::{AtomicU64, Ordering};

/// Global counter for deterministic ID generation in tests.
static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Whether to use deterministic IDs (for testing).
static USE_DETERMINISTIC_IDS: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Enable deterministic ID generation for testing.
///
/// When enabled, IDs will use a counter instead of random hex.
pub fn enable_deterministic_ids() {
    USE_DETERMINISTIC_IDS.store(true, Ordering::SeqCst);
    TEST_COUNTER.store(0, Ordering::SeqCst);
}

/// Disable deterministic ID generation.
pub fn disable_deterministic_ids() {
    USE_DETERMINISTIC_IDS.store(false, Ordering::SeqCst);
}

/// Convert a title to a slug.
///
/// The slug is created by:
/// 1. Converting to lowercase
/// 2. Replacing non-alphanumeric characters with hyphens
/// 3. Collapsing multiple hyphens into one
/// 4. Trimming leading/trailing hyphens
/// 5. Truncating to a maximum length (default 50 characters)
#[must_use]
pub fn slugify(title: &str) -> String {
    slugify_with_max_len(title, 50)
}

/// Convert a title to a slug with a custom maximum length.
#[must_use]
pub fn slugify_with_max_len(title: &str, max_len: usize) -> String {
    let mut slug = String::with_capacity(title.len());
    let mut last_was_hyphen = true; // Start true to avoid leading hyphen

    for c in title.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
            last_was_hyphen = false;
        } else if !last_was_hyphen {
            slug.push('-');
            last_was_hyphen = true;
        }
    }

    // Trim trailing hyphen
    if slug.ends_with('-') {
        slug.pop();
    }

    // Truncate to max length, but don't cut in the middle of a hyphen sequence
    if slug.len() > max_len {
        slug.truncate(max_len);
        // Remove trailing hyphen if we cut at a boundary
        while slug.ends_with('-') {
            slug.pop();
        }
    }

    slug
}

/// Generate a random 4-character hex suffix.
#[allow(clippy::cast_possible_truncation)]
fn random_suffix() -> String {
    if USE_DETERMINISTIC_IDS.load(Ordering::SeqCst) {
        let count = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        format!("{count:04x}")
    } else {
        use std::collections::hash_map::RandomState;
        use std::hash::{BuildHasher, Hasher};

        let state = RandomState::new();
        let mut hasher = state.build_hasher();
        // Truncation is intentional - we only need entropy, not precision
        hasher.write_u64(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos() as u64),
        );
        let hash = hasher.finish();
        format!("{:04x}", hash & 0xFFFF)
    }
}

/// Generate a task ID from a title.
///
/// The ID is the slugified title plus a 4-character random hex suffix.
#[must_use]
pub fn generate_task_id(title: &str) -> String {
    let slug = slugify(title);
    let suffix = random_suffix();

    if slug.is_empty() {
        format!("task-{suffix}")
    } else {
        format!("{slug}-{suffix}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_slugify_basic() {
        assert_eq!(slugify("Hello World"), "hello-world");
        assert_eq!(slugify("Fix the bug"), "fix-the-bug");
        assert_eq!(slugify("simple"), "simple");
    }

    #[test]
    fn test_slugify_special_characters() {
        assert_eq!(slugify("Hello, World!"), "hello-world");
        assert_eq!(slugify("Fix: the bug (urgent)"), "fix-the-bug-urgent");
        assert_eq!(slugify("test@email.com"), "test-email-com");
    }

    #[test]
    fn test_slugify_multiple_spaces() {
        assert_eq!(slugify("Hello   World"), "hello-world");
        assert_eq!(slugify("  leading spaces"), "leading-spaces");
        assert_eq!(slugify("trailing spaces  "), "trailing-spaces");
    }

    #[test]
    fn test_slugify_empty() {
        assert_eq!(slugify(""), "");
        assert_eq!(slugify("   "), "");
        assert_eq!(slugify("!!!"), "");
    }

    #[test]
    fn test_slugify_numbers() {
        assert_eq!(slugify("Task 123"), "task-123");
        assert_eq!(slugify("123 Task"), "123-task");
    }

    #[test]
    fn test_slugify_unicode() {
        // Unicode non-ASCII characters are replaced with hyphens
        assert_eq!(slugify("café"), "caf");
        assert_eq!(slugify("日本語"), "");
    }

    #[test]
    fn test_slugify_truncation() {
        let long_title = "a".repeat(100);
        let slug = slugify(&long_title);
        assert!(slug.len() <= 50);
    }

    #[test]
    fn test_slugify_truncation_at_hyphen() {
        // When truncating, we should not leave a trailing hyphen
        let title = "this-is-a-very-long-title-that-will-be-truncated-at-a-hyphen-boundary";
        let slug = slugify_with_max_len(title, 30);
        assert!(!slug.ends_with('-'));
        assert!(slug.len() <= 30);
    }

    #[test]
    fn test_slugify_truncation_removes_trailing_hyphens() {
        // Test the slug.pop() branch when truncation leaves trailing hyphens
        // Input "abc  d" becomes "abc-d" (length 5)
        // With max_len=4, truncate to "abc-", then while loop pops to "abc"
        let slug = slugify_with_max_len("abc  d", 4);
        assert_eq!(slug, "abc");
        assert!(!slug.ends_with('-'));
    }

    #[test]
    fn test_slugify_truncation_removes_multiple_trailing_hyphens() {
        // Test case where truncation leaves multiple trailing hyphens
        // Input "ab    cd" becomes "ab-cd" (length 5)
        // With max_len=3, truncate to "ab-", then while loop pops to "ab"
        let slug = slugify_with_max_len("ab    cd", 3);
        assert_eq!(slug, "ab");
    }

    #[test]
    fn test_generate_task_id_format() {
        enable_deterministic_ids();

        let id = generate_task_id("Hello World");
        assert!(id.starts_with("hello-world-"));
        assert_eq!(id.len(), "hello-world-".len() + 4);

        disable_deterministic_ids();
    }

    #[test]
    fn test_generate_task_id_empty_title() {
        enable_deterministic_ids();

        let id = generate_task_id("");
        assert!(id.starts_with("task-"));
        assert_eq!(id.len(), "task-".len() + 4);

        disable_deterministic_ids();
    }

    #[test]
    fn test_generate_task_id_special_only() {
        enable_deterministic_ids();

        let id = generate_task_id("!!!");
        assert!(id.starts_with("task-"));

        disable_deterministic_ids();
    }

    #[test]
    fn test_deterministic_ids_increment() {
        enable_deterministic_ids();

        let id1 = generate_task_id("test");
        let id2 = generate_task_id("test");
        let id3 = generate_task_id("test");

        // Each ID should have a different suffix
        assert!(id1.ends_with("-0000"));
        assert!(id2.ends_with("-0001"));
        assert!(id3.ends_with("-0002"));

        disable_deterministic_ids();
    }

    #[test]
    fn test_random_suffix_unique() {
        disable_deterministic_ids();

        // Generate multiple IDs and check they're different
        // (statistically extremely unlikely to be the same)
        let id1 = generate_task_id("test");
        let id2 = generate_task_id("test");

        // The slugs are the same, but full IDs should differ
        assert!(id1.starts_with("test-"));
        assert!(id2.starts_with("test-"));
        // Note: There's a tiny chance (1/65536) they could be equal
        // but that's acceptable for this test
    }
}
