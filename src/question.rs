//! Question detection and handling for interactive conversations.

use once_cell::sync::Lazy;
use regex::Regex;

/// Patterns that suggest the assistant is asking a question.
static QUESTION_PATTERNS: Lazy<Vec<Regex>> = Lazy::new(|| {
    vec![
        Regex::new(r"\?\s*$").unwrap(),  // Ends with question mark
        Regex::new(r"\?\s*\n").unwrap(), // Question mark before newline
        Regex::new(r"(?i)\bwould you like\b").unwrap(),
        Regex::new(r"(?i)\bdo you want\b").unwrap(),
        Regex::new(r"(?i)\bshould I\b").unwrap(),
        Regex::new(r"(?i)\bcan you\b.*\?").unwrap(),
        Regex::new(r"(?i)\bwhat do you think\b").unwrap(),
        Regex::new(r"(?i)\blet me know\b").unwrap(),
        Regex::new(r"(?i)\bplease confirm\b").unwrap(),
        Regex::new(r"(?i)\bplease clarify\b").unwrap(),
        Regex::new(r"(?i)\bwhich (?:one|option)\b").unwrap(),
        Regex::new(r"(?i)\bhow would you like\b").unwrap(),
        Regex::new(r"(?i)\bwhat would you prefer\b").unwrap(),
    ]
});

/// Patterns for "should I continue?" questions - auto-answered without sub-agent.
/// These are questions where the agent is asking if it should keep working.
static CONTINUE_QUESTION_PATTERNS: Lazy<Vec<Regex>> = Lazy::new(|| {
    vec![
        Regex::new(r"(?i)\bwould you like me to continue\b").unwrap(),
        Regex::new(r"(?i)\bshould I continue\b").unwrap(),
        Regex::new(r"(?i)\bdo you want me to continue\b").unwrap(),
        Regex::new(r"(?i)\bshall I continue\b").unwrap(),
        Regex::new(r"(?i)\bshould I proceed\b").unwrap(),
        Regex::new(r"(?i)\bdo you want me to proceed\b").unwrap(),
        Regex::new(r"(?i)\bwould you like me to proceed\b").unwrap(),
        Regex::new(r"(?i)\bshall I proceed\b").unwrap(),
        Regex::new(r"(?i)\bdo you want me to keep going\b").unwrap(),
        Regex::new(r"(?i)\bshould I keep going\b").unwrap(),
        Regex::new(r"(?i)\bdo you want me to do the rest\b").unwrap(),
        Regex::new(r"(?i)\bshould I do the rest\b").unwrap(),
        Regex::new(r"(?i)\bwant me to continue\b").unwrap(),
        Regex::new(r"(?i)\bwant me to proceed\b").unwrap(),
        Regex::new(r"(?i)\bwant me to keep\b").unwrap(),
    ]
});

/// Check if the text appears to be asking the user a question.
pub fn looks_like_question(text: &str) -> bool {
    QUESTION_PATTERNS.iter().any(|re| re.is_match(text))
}

/// Check if the text is asking whether to continue/proceed.
///
/// These questions can be auto-answered without consulting a sub-agent.
pub fn is_continue_question(text: &str) -> bool {
    CONTINUE_QUESTION_PATTERNS.iter().any(|re| re.is_match(text))
}

/// Truncate text to the given length by taking the last `max_chars` characters.
///
/// Returns the full text if it's already within the limit.
pub fn truncate_for_context(text: &str, max_chars: usize) -> &str {
    if text.len() <= max_chars {
        return text;
    }

    // Take the last max_chars characters
    let start = text.len().saturating_sub(max_chars);
    &text[start..]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_looks_like_question_ends_with_question_mark() {
        assert!(looks_like_question("What do you think?"));
        assert!(looks_like_question("Is this correct? "));
        assert!(looks_like_question("Hmm?\n"));
    }

    #[test]
    fn test_looks_like_question_phrases() {
        assert!(looks_like_question("Would you like me to help?"));
        assert!(looks_like_question("Do you want to proceed?"));
        assert!(looks_like_question("Should I fix this?"));
        assert!(looks_like_question("What do you think about this?"));
        assert!(looks_like_question("Please let me know if you need help."));
        assert!(looks_like_question("Please confirm the changes."));
        assert!(looks_like_question("Which option do you prefer?"));
    }

    #[test]
    fn test_looks_like_question_not_questions() {
        assert!(!looks_like_question("This is a statement."));
        assert!(!looks_like_question("I will do this."));
        assert!(!looks_like_question("The code is fixed."));
    }

    #[test]
    fn test_is_continue_question() {
        assert!(is_continue_question("Would you like me to continue?"));
        assert!(is_continue_question("Should I continue with the rest?"));
        assert!(is_continue_question("Do you want me to proceed?"));
        assert!(is_continue_question("Shall I continue?"));
        assert!(is_continue_question("Do you want me to keep going?"));
        assert!(is_continue_question("Should I do the rest?"));
        assert!(is_continue_question("Want me to continue?"));
    }

    #[test]
    fn test_is_continue_question_not_continue() {
        // These are questions but not about continuing
        assert!(!is_continue_question("What do you think?"));
        assert!(!is_continue_question("Should I use TypeScript?"));
        assert!(!is_continue_question("Do you want the old or new version?"));
    }

    #[test]
    fn test_truncate_for_context_short() {
        let text = "short text";
        assert_eq!(truncate_for_context(text, 100), "short text");
    }

    #[test]
    fn test_truncate_for_context_long() {
        let text = "a".repeat(3000);
        let truncated = truncate_for_context(&text, 2000);
        assert_eq!(truncated.len(), 2000);
    }

    #[test]
    fn test_case_insensitive() {
        assert!(looks_like_question("WOULD YOU LIKE to help?"));
        assert!(is_continue_question("SHOULD I CONTINUE?"));
    }
}
