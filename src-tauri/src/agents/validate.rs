use super::AgentError;

pub const COLORS: [&str; 9] = [
    "slate", "red", "orange", "amber", "green", "teal", "blue", "violet", "pink",
];

const NAME_MAX: usize = 40;
const MODEL_MAX: usize = 100;

pub fn name(raw: &str) -> Result<String, AgentError> {
    let name = raw.trim();
    match name.chars().count() {
        0 => Err(AgentError::new("name-empty", "Enter a name.")),
        count if count > NAME_MAX => Err(AgentError::new(
            "name-too-long",
            format!("Use at most {NAME_MAX} characters."),
        )),
        _ => Ok(name.to_string()),
    }
}

pub fn color(raw: &str) -> Result<String, AgentError> {
    if COLORS.contains(&raw) {
        Ok(raw.to_string())
    } else {
        Err(AgentError::new(
            "color-invalid",
            "Pick a colour from the list.",
        ))
    }
}

pub fn model(raw: &str) -> Result<String, AgentError> {
    let model = raw.trim();
    let count = model.chars().count();
    if count == 0 || count > MODEL_MAX || model.chars().any(char::is_whitespace) {
        return Err(AgentError::new(
            "model-invalid",
            format!("Use 1 to {MODEL_MAX} characters with no spaces."),
        ));
    }
    Ok(model.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code<T>(result: Result<T, AgentError>) -> &'static str {
        result.err().expect("expected an error").code
    }

    #[test]
    fn names_are_trimmed_and_bounded() {
        assert_eq!(name("  Test Agent ").unwrap(), "Test Agent");
        assert_eq!(code(name("   ")), "name-empty");
        assert_eq!(code(name("")), "name-empty");
        assert!(name(&"a".repeat(40)).is_ok());
        assert_eq!(code(name(&"a".repeat(41))), "name-too-long");
    }

    #[test]
    fn name_length_counts_characters_not_bytes() {
        assert!(name(&"ก".repeat(40)).is_ok());
        assert_eq!(code(name(&"ก".repeat(41))), "name-too-long");
    }

    #[test]
    fn colors_must_be_in_the_palette() {
        for color_name in COLORS {
            assert!(color(color_name).is_ok());
        }
        assert_eq!(code(color("purple")), "color-invalid");
        assert_eq!(code(color("")), "color-invalid");
        assert_eq!(code(color("Red")), "color-invalid");
    }

    #[test]
    fn models_have_no_inner_whitespace() {
        assert_eq!(model(" default ").unwrap(), "default");
        assert!(model("claude-sonnet-4-5").is_ok());
        assert_eq!(code(model("")), "model-invalid");
        assert_eq!(code(model("two words")), "model-invalid");
        assert!(model(&"m".repeat(100)).is_ok());
        assert_eq!(code(model(&"m".repeat(101))), "model-invalid");
    }
}
