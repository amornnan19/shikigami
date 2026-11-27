use serde_json::Value;

use crate::secrets::SecretString;

pub const REDACTED: &str = "[redacted]";
pub const HOLD_BACK: usize = 120;
const TOKEN_PREFIX: &str = "sk-ant-";
const TOKEN_MIN_LEN: usize = 20;

#[derive(Default)]
pub struct Redactor {
    key: Option<SecretString>,
}

impl Clone for Redactor {
    fn clone(&self) -> Self {
        Self {
            key: self
                .key
                .as_ref()
                .map(|secret| SecretString::new(secret.expose())),
        }
    }
}

fn in_key_alphabet(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_'
}

fn token_spans(text: &str) -> Vec<(usize, usize)> {
    let bytes = text.as_bytes();
    let mut spans = Vec::new();
    let mut from = 0;
    while let Some(found) = text[from..].find(TOKEN_PREFIX) {
        let start = from + found;
        let mut end = start + TOKEN_PREFIX.len();
        while end < bytes.len() && in_key_alphabet(bytes[end]) {
            end += 1;
        }
        if end - start >= TOKEN_MIN_LEN {
            spans.push((start, end));
            from = end;
        } else {
            from = start + 1;
        }
    }
    spans
}

impl Redactor {
    pub fn none() -> Self {
        Self { key: None }
    }

    pub fn for_key(key: Option<&SecretString>) -> Self {
        Self {
            key: key
                .filter(|secret| !secret.expose().is_empty())
                .map(|secret| SecretString::new(secret.expose())),
        }
    }

    pub fn hold_back(&self) -> usize {
        self.key
            .as_ref()
            .map_or(HOLD_BACK, |secret| HOLD_BACK.max(secret.expose().len()))
    }

    pub fn spans(&self, text: &str) -> Vec<(usize, usize)> {
        let mut spans = token_spans(text);
        if let Some(secret) = &self.key {
            spans.extend(
                text.match_indices(secret.expose())
                    .map(|(start, found)| (start, start + found.len())),
            );
        }
        spans.sort_unstable();
        let mut merged: Vec<(usize, usize)> = Vec::new();
        for (start, end) in spans {
            match merged.last_mut() {
                Some(last) if start <= last.1 => last.1 = last.1.max(end),
                _ => merged.push((start, end)),
            }
        }
        merged
    }

    pub fn redact(&self, text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        let mut at = 0;
        for (start, end) in self.spans(text) {
            out.push_str(&text[at..start]);
            out.push_str(REDACTED);
            at = end;
        }
        out.push_str(&text[at..]);
        out
    }

    pub fn redact_value(&self, value: &Value) -> Value {
        match value {
            Value::String(text) => Value::String(self.redact(text)),
            Value::Array(items) => {
                Value::Array(items.iter().map(|item| self.redact_value(item)).collect())
            }
            Value::Object(map) => Value::Object(
                map.iter()
                    .map(|(name, item)| (self.redact(name), self.redact_value(item)))
                    .collect(),
            ),
            other => other.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "sk-ant-api03-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

    fn redactor() -> Redactor {
        Redactor::for_key(Some(&SecretString::new(KEY)))
    }

    #[test]
    fn the_saved_key_and_any_key_shaped_token_are_redacted() {
        let text = format!("a {KEY} b sk-ant-other-token-0123456789 c sk-ant-short d");
        assert_eq!(
            redactor().redact(&text),
            "a [redacted] b [redacted] c sk-ant-short d"
        );
        assert_eq!(
            Redactor::none().redact("sk-ant-other-token-0123456789"),
            "[redacted]"
        );
    }

    #[test]
    fn a_saved_key_outside_the_token_shape_is_redacted() {
        let redactor = Redactor::for_key(Some(&SecretString::new("pass word!")));
        assert_eq!(redactor.redact("my pass word! here"), "my [redacted] here");
    }

    #[test]
    fn strings_inside_json_are_redacted() {
        let value = serde_json::json!({"a": [KEY, 1], "b": {"c": format!("x{KEY}")}});
        let text = redactor().redact_value(&value).to_string();
        assert!(!text.contains("sk-ant"));
        assert!(text.contains(REDACTED));
    }

    #[test]
    fn object_keys_inside_json_are_redacted() {
        let value = serde_json::json!({KEY: "v", "plain": {"nested": 1, "sk-ant-other-token-0123456789": [KEY]}});
        let redacted = redactor().redact_value(&value);
        assert_eq!(
            redacted,
            serde_json::json!({REDACTED: "v", "plain": {"nested": 1, REDACTED: [REDACTED]}})
        );
        assert!(!redacted.to_string().contains("sk-ant"));
    }
}
