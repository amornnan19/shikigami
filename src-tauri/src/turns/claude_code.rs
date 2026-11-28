use std::collections::HashMap;
use std::path::Path;

use serde_json::Value;

use super::group::AccessLevel;
use super::redact::Redactor;

pub const DETAIL_LIMIT: usize = 4096;
const TITLE_LIMIT: usize = 120;
const TITLE_FIELDS: [&str; 4] = ["file_path", "pattern", "command", "url"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Session {
    New(String),
    Resume(String),
}

const ISOLATION_FLAGS: [&str; 6] = [
    "--setting-sources",
    "project",
    "--strict-mcp-config",
    "--permission-mode",
    "manual",
    "--permission-prompts",
];

pub struct GroupFlags<'a> {
    pub instructions_file: Option<&'a Path>,
    pub snapshot_off: bool,
    pub access: AccessLevel,
}

pub fn arguments(model: &str, session: &Session, group: Option<&GroupFlags>) -> Vec<String> {
    let mut args: Vec<String> = [
        "-p",
        "--output-format",
        "stream-json",
        "--verbose",
        "--include-partial-messages",
    ]
    .into_iter()
    .chain(ISOLATION_FLAGS)
    .chain(["none"])
    .map(str::to_owned)
    .collect();
    if model != "default" {
        args.push("--model".to_owned());
        args.push(model.to_owned());
    }
    match session {
        Session::New(id) => {
            args.push("--session-id".to_owned());
            args.push(id.clone());
        }
        Session::Resume(id) => {
            args.push("--resume".to_owned());
            args.push(id.clone());
        }
    }
    if let Some(group) = group {
        if let Some(at) = args.iter().position(|arg| arg == "--setting-sources") {
            args[at + 1] = String::new();
        }
        if let Some(at) = args.iter().position(|arg| arg == "--permission-mode") {
            args[at + 1] = group.access.permission_mode().to_owned();
        }
        if let Some(file) = group.instructions_file {
            args.push("--append-system-prompt-file".to_owned());
            args.push(file.to_string_lossy().into_owned());
        }
        if group.snapshot_off {
            args.push("--system-prompt-snapshot".to_owned());
            args.push("off".to_owned());
        }
    }
    args
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepStart {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub input: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Parsed {
    Session(String),
    TextDelta {
        item_id: String,
        text: String,
    },
    TextSet {
        item_id: String,
        text: String,
    },
    StepStarted(StepStart),
    StepEnded {
        step_id: String,
        failed: bool,
        output: String,
    },
    Result {
        is_error: bool,
        text: String,
    },
}

#[derive(Default)]
struct MessageTexts {
    items: Vec<String>,
    finalised: usize,
}

#[derive(Default)]
pub struct StreamParser {
    redactor: Redactor,
    message_id: Option<String>,
    streamed: HashMap<(String, u64), String>,
    messages: HashMap<String, MessageTexts>,
    steps: HashMap<String, String>,
}

fn new_id() -> String {
    uuid::Uuid::now_v7().to_string()
}

pub fn truncate(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        return text.to_owned();
    }
    let mut end = limit;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end])
}

fn title_of(kind: &str, input: &Value) -> String {
    let summary = TITLE_FIELDS
        .iter()
        .filter_map(|field| input.get(field).and_then(Value::as_str))
        .filter_map(|value| value.lines().map(str::trim).find(|line| !line.is_empty()))
        .next();
    match summary {
        Some(line) if line.chars().count() > TITLE_LIMIT => {
            let cut: String = line.chars().take(TITLE_LIMIT).collect();
            format!("{cut}…")
        }
        Some(line) => line.to_owned(),
        None => kind.to_owned(),
    }
}

fn result_text(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter_map(|block| block.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

impl StreamParser {
    pub fn new(redactor: Redactor) -> Self {
        Self {
            redactor,
            ..Self::default()
        }
    }

    pub fn line(&mut self, line: &str) -> Vec<Parsed> {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            return Vec::new();
        };
        if !value.get("parent_tool_use_id").is_none_or(Value::is_null) {
            return Vec::new();
        }
        match value.get("type").and_then(Value::as_str) {
            Some("system") => Self::system(&value),
            Some("stream_event") => self.stream_event(&value),
            Some("assistant") => self.assistant(&value),
            Some("user") => self.user(&value),
            Some("result") => Self::result(&value),
            _ => Vec::new(),
        }
    }

    fn system(value: &Value) -> Vec<Parsed> {
        if value.get("subtype").and_then(Value::as_str) != Some("init") {
            return Vec::new();
        }
        value
            .get("session_id")
            .and_then(Value::as_str)
            .map(|id| vec![Parsed::Session(id.to_owned())])
            .unwrap_or_default()
    }

    fn message_key(&self, value: &Value) -> String {
        value
            .get("api_message_id")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| self.message_id.clone())
            .unwrap_or_default()
    }

    fn stream_event(&mut self, value: &Value) -> Vec<Parsed> {
        let Some(event) = value.get("event") else {
            return Vec::new();
        };
        let index = event.get("index").and_then(Value::as_u64).unwrap_or(0);
        match event.get("type").and_then(Value::as_str) {
            Some("message_start") => {
                self.message_id = event
                    .pointer("/message/id")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                Vec::new()
            }
            Some("content_block_start") => {
                let is_text =
                    event.pointer("/content_block/type").and_then(Value::as_str) == Some("text");
                if is_text {
                    self.streamed_item(self.message_key(value), index);
                }
                Vec::new()
            }
            Some("content_block_delta") => {
                let delta = event.get("delta");
                let is_text =
                    delta.and_then(|d| d.get("type")).and_then(Value::as_str) == Some("text_delta");
                let text = delta.and_then(|d| d.get("text")).and_then(Value::as_str);
                match (is_text, text) {
                    (true, Some(text)) if !text.is_empty() => vec![Parsed::TextDelta {
                        item_id: self.streamed_item(self.message_key(value), index),
                        text: text.to_owned(),
                    }],
                    _ => Vec::new(),
                }
            }
            _ => Vec::new(),
        }
    }

    fn streamed_item(&mut self, message: String, index: u64) -> String {
        if let Some(id) = self.streamed.get(&(message.clone(), index)) {
            return id.clone();
        }
        let id = new_id();
        self.streamed.insert((message.clone(), index), id.clone());
        self.messages
            .entry(message)
            .or_default()
            .items
            .push(id.clone());
        id
    }

    fn assistant(&mut self, value: &Value) -> Vec<Parsed> {
        let Some(message) = value.get("message") else {
            return Vec::new();
        };
        let message_id = message
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let Some(blocks) = message.get("content").and_then(Value::as_array) else {
            return Vec::new();
        };
        let mut parsed = Vec::new();
        for block in blocks {
            match block.get("type").and_then(Value::as_str) {
                Some("text") => {
                    let text = block
                        .get("text")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    parsed.extend(self.final_text(&message_id, text));
                }
                Some("tool_use") => parsed.extend(self.tool_use(block)),
                _ => {}
            }
        }
        parsed
    }

    fn final_text(&mut self, message_id: &str, text: &str) -> Option<Parsed> {
        let texts = self.messages.entry(message_id.to_owned()).or_default();
        let position = texts.finalised;
        texts.finalised += 1;
        let existing = texts.items.get(position).cloned();
        if existing.is_none() && text.is_empty() {
            return None;
        }
        let item_id = existing.unwrap_or_else(|| {
            let id = new_id();
            texts.items.push(id.clone());
            id
        });
        Some(Parsed::TextSet {
            item_id,
            text: text.to_owned(),
        })
    }

    fn tool_use(&mut self, block: &Value) -> Option<Parsed> {
        let tool_use_id = block.get("id").and_then(Value::as_str)?;
        if self.steps.contains_key(tool_use_id) {
            return None;
        }
        let kind = block.get("name").and_then(Value::as_str).unwrap_or("tool");
        let input = self
            .redactor
            .redact_value(block.get("input").unwrap_or(&Value::Null));
        let id = new_id();
        self.steps.insert(tool_use_id.to_owned(), id.clone());
        Some(Parsed::StepStarted(StepStart {
            id,
            kind: kind.to_owned(),
            title: title_of(kind, &input),
            input: truncate(&input.to_string(), DETAIL_LIMIT),
        }))
    }

    fn user(&mut self, value: &Value) -> Vec<Parsed> {
        let Some(blocks) = value.pointer("/message/content").and_then(Value::as_array) else {
            return Vec::new();
        };
        blocks
            .iter()
            .filter(|block| block.get("type").and_then(Value::as_str) == Some("tool_result"))
            .filter_map(|block| {
                let tool_use_id = block.get("tool_use_id").and_then(Value::as_str)?;
                let step_id = self.steps.remove(tool_use_id)?;
                Some(Parsed::StepEnded {
                    step_id,
                    failed: block.get("is_error").and_then(Value::as_bool) == Some(true),
                    output: truncate(
                        &self
                            .redactor
                            .redact(&block.get("content").map(result_text).unwrap_or_default()),
                        DETAIL_LIMIT,
                    ),
                })
            })
            .collect()
    }

    fn result(value: &Value) -> Vec<Parsed> {
        let is_error = value.get("is_error").and_then(Value::as_bool) == Some(true);
        let text = value
            .get("result")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        vec![Parsed::Result { is_error, text }]
    }
}

#[cfg(test)]
const ISOLATION: [&str; 7] = [
    "--setting-sources",
    "project",
    "--strict-mcp-config",
    "--permission-mode",
    "manual",
    "--permission-prompts",
    "none",
];

#[cfg(test)]
pub fn assert_isolated(args: &[String]) {
    let at = args
        .iter()
        .position(|arg| arg == "--setting-sources")
        .expect("--setting-sources missing");
    assert_eq!(args[at], ISOLATION[0]);
    assert!(args[at + 1] == "project" || args[at + 1].is_empty());
    assert_eq!(args[at + 2..at + ISOLATION.len()], ISOLATION[2..]);
    for forbidden in ["bypassPermissions", "acceptEdits", "auto", "dontAsk"] {
        assert!(!args.iter().any(|arg| arg == forbidden), "{forbidden}");
    }
    assert!(!args.iter().any(|arg| arg.starts_with("--dangerously")));
    assert!(!args
        .iter()
        .any(|arg| arg.starts_with("--allow-dangerously")));
    assert!(!args
        .iter()
        .any(|arg| arg.to_lowercase().contains("allowedtools")));
    assert!(!args
        .iter()
        .any(|arg| arg.to_lowercase().contains("allowed-tools")));
    assert_eq!(
        args.iter()
            .filter(|arg| *arg == "--permission-mode")
            .count(),
        1
    );
    assert_eq!(args[at + 1].split(',').count(), 1);
    assert!(!args[at + 1].contains("user"));
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASIC: &str = include_str!("../../tests/fixtures/claude-stream-basic.jsonl");
    const DENIED: &str = include_str!("../../tests/fixtures/claude-stream-denied.jsonl");

    fn parse(source: &str) -> Vec<Parsed> {
        let mut parser = StreamParser::default();
        source.lines().flat_map(|line| parser.line(line)).collect()
    }

    fn streamed_text(parsed: &[Parsed], item: &str) -> String {
        parsed
            .iter()
            .filter_map(|event| match event {
                Parsed::TextDelta { item_id, text } if item_id == item => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn arguments_pass_the_isolation_flags_the_model_and_the_session() {
        let first = arguments("default", &Session::New("abc".into()), None);
        assert_eq!(
            first,
            [
                "-p",
                "--output-format",
                "stream-json",
                "--verbose",
                "--include-partial-messages",
                "--setting-sources",
                "project",
                "--strict-mcp-config",
                "--permission-mode",
                "manual",
                "--permission-prompts",
                "none",
                "--session-id",
                "abc"
            ]
        );
        let later = arguments("claude-haiku-5-5", &Session::Resume("abc".into()), None);
        assert_eq!(
            &later[12..],
            ["--model", "claude-haiku-5-5", "--resume", "abc"]
        );
    }

    #[test]
    fn every_argument_list_keeps_the_isolation_flags_and_no_widening_value() {
        for model in ["default", "haiku"] {
            for session in [Session::New("abc".into()), Session::Resume("abc".into())] {
                let file = Path::new("/data/tmp/instructions.md");
                for group in [
                    None,
                    Some(GroupFlags {
                        instructions_file: Some(file),
                        snapshot_off: true,
                        access: AccessLevel::STANDARD,
                    }),
                ] {
                    let args = arguments(model, &session, group.as_ref());
                    assert_isolated(&args);
                }
            }
        }
    }

    #[test]
    fn a_group_turn_loads_no_setting_sources_and_takes_its_instructions_from_a_file() {
        let file = Path::new("/data/tmp/instructions.md");
        let args = arguments(
            "haiku",
            &Session::Resume("abc".into()),
            Some(&GroupFlags {
                instructions_file: Some(file),
                snapshot_off: true,
                access: AccessLevel::STANDARD,
            }),
        );
        assert_eq!(args[5], "--setting-sources");
        assert_eq!(args[6], "");
        assert_eq!(
            &args[12..],
            [
                "--model",
                "haiku",
                "--resume",
                "abc",
                "--append-system-prompt-file",
                "/data/tmp/instructions.md",
                "--system-prompt-snapshot",
                "off"
            ]
        );
        let bare = arguments(
            "haiku",
            &Session::Resume("abc".into()),
            Some(&GroupFlags {
                instructions_file: None,
                snapshot_off: false,
                access: AccessLevel::STANDARD,
            }),
        );
        assert_eq!(&bare[12..], ["--model", "haiku", "--resume", "abc"]);
        assert_eq!(bare[6], "");
        let plain = arguments("haiku", &Session::Resume("abc".into()), None);
        assert_eq!(plain[6], "project");
    }

    #[test]
    fn the_basic_fixture_takes_the_session_streams_text_and_ends_the_turn() {
        let parsed = parse(BASIC);
        assert!(matches!(&parsed[0], Parsed::Session(id) if !id.is_empty()));
        let Some(Parsed::TextSet { item_id, text }) = parsed
            .iter()
            .find(|event| matches!(event, Parsed::TextSet { .. }))
        else {
            panic!("no authoritative text");
        };
        assert_eq!(text, "pong");
        assert_eq!(&streamed_text(&parsed, item_id), "pong");
        assert_eq!(
            parsed.last(),
            Some(&Parsed::Result {
                is_error: false,
                text: "pong".to_owned()
            })
        );
    }

    #[test]
    fn streamed_text_assembles_into_the_authoritative_text() {
        let parsed = parse(DENIED);
        let sets: Vec<(&String, &String)> = parsed
            .iter()
            .filter_map(|event| match event {
                Parsed::TextSet { item_id, text } => Some((item_id, text)),
                _ => None,
            })
            .collect();
        assert_eq!(sets.len(), 1);
        let (item_id, text) = sets[0];
        assert!(text.chars().count() > 20);
        assert_eq!(&streamed_text(&parsed, item_id), text);
        assert!(
            parsed
                .iter()
                .filter(|e| matches!(e, Parsed::TextDelta { .. }))
                .count()
                > 5
        );
    }

    fn started_steps(parsed: &[Parsed]) -> Vec<&StepStart> {
        parsed
            .iter()
            .filter_map(|event| match event {
                Parsed::StepStarted(step) => Some(step),
                _ => None,
            })
            .collect()
    }

    fn ended_steps(parsed: &[Parsed]) -> Vec<(&String, bool, &String)> {
        parsed
            .iter()
            .filter_map(|event| match event {
                Parsed::StepEnded {
                    step_id,
                    failed,
                    output,
                } => Some((step_id, *failed, output)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn tool_uses_start_steps_with_their_titles_and_failed_results_end_them_failed() {
        let parsed = parse(DENIED);
        let started = started_steps(&parsed);
        assert_eq!(started.len(), 2);
        assert!(started.iter().all(|step| step.kind == "Bash"));
        assert!(started[0].title.starts_with("ls -la"));
        assert!(started[1].title.starts_with("/bin/ls -la"));
        assert!(started[0].input.contains("\"command\""));
        let ended = ended_steps(&parsed);
        assert_eq!(ended.len(), 2);
        assert_eq!(ended[0].0, &started[0].id);
        assert_eq!(ended[1].0, &started[1].id);
        assert!(ended.iter().all(|(_, failed, _)| *failed));
        assert!(ended[0].2.contains("command not found"));
        assert!(ended[1].2.contains("denied"));
    }

    #[test]
    fn a_tool_result_without_an_error_ends_the_step_done() {
        let succeeding = DENIED.replace("\"is_error\":true", "\"is_error\":false");
        let parsed = parse(&succeeding);
        let ended = ended_steps(&parsed);
        assert_eq!(ended.len(), 2);
        assert!(ended.iter().all(|(_, failed, _)| !*failed));
    }

    #[test]
    fn an_error_result_carries_the_result_text() {
        let line = r#"{"type":"result","subtype":"error_during_execution","is_error":true,"result":"Boom"}"#;
        assert_eq!(
            StreamParser::default().line(line),
            vec![Parsed::Result {
                is_error: true,
                text: "Boom".to_owned()
            }]
        );
    }

    #[test]
    fn unknown_kinds_and_broken_lines_are_ignored() {
        let mut parser = StreamParser::default();
        for line in [
            r#"{"type":"rate_limit_event","x":1}"#,
            r#"{"type":"system","subtype":"hook_started"}"#,
            r#"{"type":"something-new"}"#,
            "not json",
            "",
            r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"hm"}}}"#,
            r#"{"type":"assistant","parent_tool_use_id":"toolu_x","message":{"id":"m","content":[{"type":"text","text":"from a subagent"}]}}"#,
        ] {
            assert!(parser.line(line).is_empty(), "{line}");
        }
    }

    #[test]
    fn titles_use_the_first_known_input_field_or_the_tool_name() {
        let read = serde_json::json!({"file_path": "/a/b.txt"});
        assert_eq!(title_of("Read", &read), "/a/b.txt");
        let grep = serde_json::json!({"pattern": "foo", "path": "/x"});
        assert_eq!(title_of("Grep", &grep), "foo");
        let fetch = serde_json::json!({"url": "https://example.com"});
        assert_eq!(title_of("WebFetch", &fetch), "https://example.com");
        let multi = serde_json::json!({"command": "\n  echo one\necho two"});
        assert_eq!(title_of("Bash", &multi), "echo one");
        assert_eq!(title_of("Task", &serde_json::json!({"x": 1})), "Task");
        let long = serde_json::json!({"command": "x".repeat(300)});
        assert_eq!(title_of("Bash", &long).chars().count(), TITLE_LIMIT + 1);
    }

    #[test]
    fn truncate_keeps_whole_characters_and_marks_the_cut() {
        assert_eq!(truncate("abc", 4), "abc");
        assert_eq!(truncate("abcdef", 3), "abc…");
        assert_eq!(truncate("ก", 1), "…");
        assert_eq!(truncate("กข", 3), "ก…");
    }

    #[test]
    fn text_without_streaming_gets_a_new_item() {
        let line =
            r#"{"type":"assistant","message":{"id":"m1","content":[{"type":"text","text":"hi"}]}}"#;
        let parsed = StreamParser::default().line(line);
        assert!(matches!(&parsed[0], Parsed::TextSet { text, .. } if text == "hi"));
    }
}
