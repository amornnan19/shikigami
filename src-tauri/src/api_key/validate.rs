use super::ApiKeyError;
use crate::secrets::SecretString;

const PREFIX: &str = "sk-ant-";
const LENGTH: std::ops::RangeInclusive<usize> = 20..=300;

pub fn key(input: &str) -> Result<SecretString, ApiKeyError> {
    let trimmed = input.trim();
    let has_bad_char = trimmed.chars().any(|c| c.is_whitespace() || c.is_control());
    if trimmed.starts_with(PREFIX) && LENGTH.contains(&trimmed.chars().count()) && !has_bad_char {
        Ok(SecretString::new(trimmed))
    } else {
        Err(ApiKeyError::key_invalid())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn accepted(input: &str) -> bool {
        key(input).is_ok()
    }

    #[test]
    fn accepts_a_valid_key_and_trims_it() {
        assert!(accepted("sk-ant-test-0000000000000000"));
        let trimmed = key("  sk-ant-test-0000000000000000\n").unwrap();
        assert_eq!(trimmed.expose(), "sk-ant-test-0000000000000000");
    }

    #[test]
    fn refuses_a_missing_prefix() {
        assert!(!accepted("sk-xyz-test-0000000000000000"));
        assert!(!accepted(" x sk-ant-test-0000000000000000"));
    }

    #[test]
    fn refuses_a_key_that_is_too_short() {
        assert!(!accepted("sk-ant-012345678901"));
        assert!(accepted("sk-ant-0123456789012"));
    }

    #[test]
    fn refuses_a_key_that_is_too_long() {
        assert!(accepted(&format!("sk-ant-{}", "a".repeat(293))));
        assert!(!accepted(&format!("sk-ant-{}", "a".repeat(294))));
    }

    #[test]
    fn refuses_inner_whitespace_and_control_characters() {
        assert!(!accepted("sk-ant-test 0000000000000000"));
        assert!(!accepted("sk-ant-test-0000\n0000000000"));
        assert!(!accepted("sk-ant-test-0000\t0000000000"));
        assert!(!accepted("sk-ant-test-0000\u{7}0000000000"));
    }

    #[test]
    fn the_error_does_not_contain_the_key() {
        let error = key("not-a-key sk-ant-secret-secret-secret").unwrap_err();
        assert!(!error.message.contains("secret"));
    }
}
