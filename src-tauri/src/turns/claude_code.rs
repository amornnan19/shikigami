use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde_json::Value;

use super::group::AccessLevel;
use super::redact::Redactor;

pub const DETAIL_LIMIT: usize = 4096;
const TITLE_LIMIT: usize = 120;
const TITLE_FIELDS: [&str; 4] = ["file_path", "pattern", "command", "url"];
const SUBAGENT_TOOLS: [&str; 2] = ["Agent", "Task"];
const SUBAGENT_DEFAULT_TYPE: &str = "general-purpose";
const SUBAGENT_TYPE_LIMIT: usize = 60;
const SUBAGENT_LABEL_LIMIT: usize = 80;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Session {
    New(String),
    Resume(String),
}

const ISOLATION_FLAGS: [&str; 6] = [
    "--setting-sources",
    "",
    "--strict-mcp-config",
    "--permission-mode",
    "manual",
    "--permission-prompts",
];

pub struct GroupFlags {
    pub snapshot_off: bool,
    pub access: AccessLevel,
}

pub fn arguments(
    model: &str,
    session: &Session,
    group: Option<&GroupFlags>,
    instructions_file: Option<&Path>,
) -> Vec<String> {
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
    if let Some(file) = instructions_file {
        args.push("--append-system-prompt-file".to_owned());
        args.push(file.to_string_lossy().into_owned());
    }
    if let Some(group) = group {
        if let Some(at) = args.iter().position(|arg| arg == "--permission-mode") {
            args[at + 1] = group.access.permission_mode().to_owned();
        }
        if group.snapshot_off {
            args.push("--system-prompt-snapshot".to_owned());
            args.push("off".to_owned());
        }
    }
    args
}

pub fn is_model_or_tool_event(line: &str) -> bool {
    serde_json::from_str::<Value>(line)
        .ok()
        .and_then(|value| value.get("type").and_then(Value::as_str).map(str::to_owned))
        .is_some_and(|kind| matches!(kind.as_str(), "assistant" | "user" | "stream_event"))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepStart {
    pub id: String,
    pub parent_step_id: Option<String>,
    pub kind: String,
    pub title: String,
    pub input: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Parsed {
    Tools(Vec<String>),
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
    StepBackground {
        step_id: String,
        output: String,
    },
    Compacted,
    Result {
        is_error: bool,
        text: String,
        api_error_status: Option<u64>,
    },
}

const SIGN_IN_MARKERS: [&str; 5] = [
    "authentication_error",
    "invalid x-api-key",
    "invalid api key",
    "not logged in",
    "please run /login",
];
const TOKEN_EXPIRED: &str = "oauth token has expired";
const UNAUTHORIZED: u64 = 401;

pub fn is_sign_in_failure(error_text: &str, api_error_status: Option<u64>) -> bool {
    if api_error_status == Some(UNAUTHORIZED) {
        return true;
    }
    let lowered = error_text.to_lowercase();
    SIGN_IN_MARKERS
        .iter()
        .chain([&TOKEN_EXPIRED])
        .any(|marker| lowered.contains(marker))
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
    step_ids: HashMap<String, String>,
    subagents: HashSet<String>,
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

fn first_chars(text: &str, limit: usize) -> String {
    text.chars().take(limit).collect()
}

fn subagent_label(input: &Value) -> (String, String) {
    let field = |name: &str| {
        input
            .get(name)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
    };
    let kind = first_chars(
        field("subagent_type").unwrap_or(SUBAGENT_DEFAULT_TYPE),
        SUBAGENT_TYPE_LIMIT,
    );
    let task = field("description")
        .or_else(|| field("prompt"))
        .map(|text| first_chars(text, SUBAGENT_LABEL_LIMIT))
        .unwrap_or_default();
    (kind, task)
}

fn subagent_input(kind: &str, task: &str, input: &Value) -> String {
    let quoted = |text: &str| Value::String(text.to_owned()).to_string();
    let prompt = input
        .get("prompt")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let build = |prompt: &str| {
        format!(
            "{{\"subagent_type\":{},\"description\":{},\"prompt\":{}}}",
            quoted(kind),
            quoted(task),
            quoted(prompt)
        )
    };
    let mut keep = prompt.chars().count();
    let mut shown = prompt.to_owned();
    loop {
        let json = build(&shown);
        if json.len() <= DETAIL_LIMIT || keep == 0 {
            return json;
        }
        keep = keep * 3 / 4;
        shown = format!("{}…", first_chars(prompt, keep));
    }
}

pub fn is_subagent_kind(kind: &str) -> bool {
    SUBAGENT_TOOLS.contains(&kind)
}

pub fn subagent_parts(detail: &str) -> (Option<String>, Option<String>) {
    let (input, result) = match detail.split_once("\n\n") {
        Some((input, result)) => (input, Some(result)),
        None => (detail, None),
    };
    let task = serde_json::from_str::<Value>(input)
        .ok()
        .and_then(|value| {
            value
                .get("prompt")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .filter(|prompt| !prompt.is_empty());
    let result = result.filter(|text| !text.is_empty()).map(str::to_owned);
    (task, result)
}

fn is_async_launch(value: &Value, text: &str) -> bool {
    value
        .pointer("/tool_use_result/status")
        .and_then(Value::as_str)
        == Some("async_launched")
        || text.contains("async_launched")
        || text.starts_with("Async agent launched")
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
        match value.get("parent_tool_use_id") {
            None | Some(Value::Null) => match value.get("type").and_then(Value::as_str) {
                Some("system") => Self::system(&value),
                Some("stream_event") => self.stream_event(&value),
                Some("assistant") => self.assistant(&value, None),
                Some("user") => self.user(&value),
                Some("result") => Self::result(&value),
                _ => Vec::new(),
            },
            Some(Value::String(parent)) => match value.get("type").and_then(Value::as_str) {
                Some("assistant") => self.assistant(&value, Some(parent)),
                Some("user") => self.user(&value),
                _ => Vec::new(),
            },
            Some(_) => Vec::new(),
        }
    }

    fn system(value: &Value) -> Vec<Parsed> {
        match value.get("subtype").and_then(Value::as_str) {
            Some("init") => {
                let tools = value
                    .get("tools")
                    .and_then(Value::as_array)
                    .map(|tools| {
                        tools
                            .iter()
                            .filter_map(Value::as_str)
                            .map(str::to_owned)
                            .collect()
                    })
                    .unwrap_or_default();
                let mut parsed = vec![Parsed::Tools(tools)];
                parsed.extend(
                    value
                        .get("session_id")
                        .and_then(Value::as_str)
                        .map(|id| Parsed::Session(id.to_owned())),
                );
                parsed
            }
            Some("compact_boundary") => vec![Parsed::Compacted],
            _ => Vec::new(),
        }
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

    fn assistant(&mut self, value: &Value, parent: Option<&str>) -> Vec<Parsed> {
        let parent_step_id = match parent {
            Some(tool_use_id) => match self.step_ids.get(tool_use_id) {
                Some(id) => Some(id.clone()),
                None => return Vec::new(),
            },
            None => None,
        };
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
                Some("tool_use") => parsed.extend(self.tool_use(block, parent_step_id.clone())),
                _ if parent_step_id.is_some() => {}
                Some("text") => {
                    let text = block
                        .get("text")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    parsed.extend(self.final_text(&message_id, text));
                }
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

    fn tool_use(&mut self, block: &Value, parent_step_id: Option<String>) -> Option<Parsed> {
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
        self.step_ids.insert(tool_use_id.to_owned(), id.clone());
        let (title, input) = if is_subagent_kind(kind) {
            self.subagents.insert(tool_use_id.to_owned());
            let (subagent, task) = subagent_label(&input);
            let json = subagent_input(&subagent, &task, &input);
            let title = if task.is_empty() { subagent } else { task };
            (title, json)
        } else {
            (
                title_of(kind, &input),
                truncate(&input.to_string(), DETAIL_LIMIT),
            )
        };
        Some(Parsed::StepStarted(StepStart {
            id,
            parent_step_id,
            kind: kind.to_owned(),
            title,
            input,
        }))
    }

    fn user(&mut self, value: &Value) -> Vec<Parsed> {
        let Some(blocks) = value.pointer("/message/content").and_then(Value::as_array) else {
            return Vec::new();
        };
        let mut parsed = Vec::new();
        for block in blocks {
            if block.get("type").and_then(Value::as_str) != Some("tool_result") {
                continue;
            }
            let Some(tool_use_id) = block.get("tool_use_id").and_then(Value::as_str) else {
                continue;
            };
            let Some(step_id) = self.steps.remove(tool_use_id) else {
                continue;
            };
            let raw = block.get("content").map(result_text).unwrap_or_default();
            let output = truncate(&self.redactor.redact(&raw), DETAIL_LIMIT);
            let failed = block.get("is_error").and_then(Value::as_bool) == Some(true);
            if !failed && self.subagents.contains(tool_use_id) && is_async_launch(value, &raw) {
                parsed.push(Parsed::StepBackground { step_id, output });
            } else {
                parsed.push(Parsed::StepEnded {
                    step_id,
                    failed,
                    output,
                });
            }
        }
        parsed
    }

    fn result(value: &Value) -> Vec<Parsed> {
        let is_error = value.get("is_error").and_then(Value::as_bool) == Some(true);
        let text = value
            .get("result")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let api_error_status = value.get("api_error_status").and_then(Value::as_u64);
        vec![Parsed::Result {
            is_error,
            text,
            api_error_status,
        }]
    }
}

#[cfg(test)]
const ISOLATION: [&str; 7] = [
    "--setting-sources",
    "",
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
    assert!(args[at + 1].is_empty());
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
    use crate::secrets::SecretString;

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
        let first = arguments("default", &Session::New("abc".into()), None, None);
        assert_eq!(
            first,
            [
                "-p",
                "--output-format",
                "stream-json",
                "--verbose",
                "--include-partial-messages",
                "--setting-sources",
                "",
                "--strict-mcp-config",
                "--permission-mode",
                "manual",
                "--permission-prompts",
                "none",
                "--session-id",
                "abc"
            ]
        );
        let later = arguments(
            "claude-haiku-5-5",
            &Session::Resume("abc".into()),
            None,
            None,
        );
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
                        snapshot_off: true,
                        access: AccessLevel::STANDARD,
                    }),
                ] {
                    for instructions in [None, Some(file)] {
                        let args = arguments(model, &session, group.as_ref(), instructions);
                        assert_isolated(&args);
                    }
                }
            }
        }
    }

    #[test]
    fn every_turn_loads_no_setting_sources_and_takes_its_instructions_from_a_file() {
        let file = Path::new("/data/tmp/instructions.md");
        let args = arguments(
            "haiku",
            &Session::Resume("abc".into()),
            Some(&GroupFlags {
                snapshot_off: true,
                access: AccessLevel::STANDARD,
            }),
            Some(file),
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
                snapshot_off: false,
                access: AccessLevel::STANDARD,
            }),
            None,
        );
        assert_eq!(&bare[12..], ["--model", "haiku", "--resume", "abc"]);
        assert_eq!(bare[6], "");
        let plain = arguments("haiku", &Session::Resume("abc".into()), None, Some(file));
        assert_eq!(plain[6], "");
        assert_eq!(
            &plain[12..],
            [
                "--model",
                "haiku",
                "--resume",
                "abc",
                "--append-system-prompt-file",
                "/data/tmp/instructions.md"
            ]
        );
    }

    #[test]
    fn the_basic_fixture_takes_the_session_streams_text_and_ends_the_turn() {
        let parsed = parse(BASIC);
        assert!(
            matches!(&parsed[0], Parsed::Tools(tools) if tools.contains(&"Read".to_owned())
            && !tools.contains(&"NotebookEdit".to_owned()))
        );
        assert!(matches!(&parsed[1], Parsed::Session(id) if !id.is_empty()));
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
                text: "pong".to_owned(),
                api_error_status: None
            })
        );
    }

    #[test]
    fn an_init_event_reports_its_tools_before_the_session_and_a_missing_list_is_empty() {
        let with = parse(
            r#"{"type":"system","subtype":"init","session_id":"s","tools":["Read","Edit","Write","Bash"]}"#,
        );
        assert_eq!(
            with,
            [
                Parsed::Tools(vec![
                    "Read".to_owned(),
                    "Edit".to_owned(),
                    "Write".to_owned(),
                    "Bash".to_owned()
                ]),
                Parsed::Session("s".to_owned())
            ]
        );
        let without = parse(r#"{"type":"system","subtype":"init","session_id":"s"}"#);
        assert_eq!(without[0], Parsed::Tools(Vec::new()));
        let no_session = parse(r#"{"type":"system","subtype":"init","tools":["Read"]}"#);
        assert_eq!(no_session, [Parsed::Tools(vec!["Read".to_owned()])]);
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

    fn lines(values: &[serde_json::Value]) -> Vec<String> {
        values.iter().map(ToString::to_string).collect()
    }

    fn parse_values(parser: &mut StreamParser, values: &[serde_json::Value]) -> Vec<Parsed> {
        lines(values)
            .iter()
            .flat_map(|line| parser.line(line))
            .collect()
    }

    fn subagent_stream() -> Vec<serde_json::Value> {
        vec![
            serde_json::json!({"type":"assistant","message":{"id":"m1","content":[{"type":"tool_use","id":"toolu_agent","name":"Agent","input":{"subagent_type":"Explore","description":"Look around","prompt":"List the files"}}]}}),
            serde_json::json!({"type":"user","parent_tool_use_id":"toolu_agent","message":{"content":[{"type":"text","text":"List the files"}]}}),
            serde_json::json!({"type":"assistant","parent_tool_use_id":"toolu_agent","message":{"id":"m2","content":[{"type":"text","text":"on it"},{"type":"tool_use","id":"toolu_inner","name":"Bash","input":{"command":"ls"}}]}}),
            serde_json::json!({"type":"user","parent_tool_use_id":"toolu_agent","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_inner","content":"a.txt","is_error":false}]}}),
            serde_json::json!({"type":"stream_event","parent_tool_use_id":"toolu_agent","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"inner words"}}}),
            serde_json::json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_agent","content":[{"type":"text","text":"found a.txt"}],"is_error":false}]}}),
        ]
    }

    #[test]
    fn a_subagent_step_carries_its_type_and_task_and_inner_steps_point_at_it() {
        let mut parser = StreamParser::default();
        let parsed = parse_values(&mut parser, &subagent_stream());
        let started = started_steps(&parsed);
        assert_eq!(started.len(), 2);
        assert_eq!(started[0].kind, "Agent");
        assert_eq!(started[0].parent_step_id, None);
        assert_eq!(started[0].title, "Look around");
        assert!(started[0].input.starts_with(
            r#"{"subagent_type":"Explore","description":"Look around","prompt":"List the files"}"#
        ));
        assert_eq!(started[1].kind, "Bash");
        assert_eq!(
            started[1].parent_step_id.as_deref(),
            Some(started[0].id.as_str())
        );
        let ended = ended_steps(&parsed);
        assert_eq!(ended.len(), 2);
        assert_eq!(ended[0].0, &started[1].id);
        assert_eq!(ended[0].2, "a.txt");
        assert_eq!(ended[1].0, &started[0].id);
        assert_eq!(ended[1].2, "found a.txt");
        assert!(parsed
            .iter()
            .all(|event| !matches!(event, Parsed::TextDelta { .. } | Parsed::TextSet { .. })));
    }

    #[test]
    fn a_subagent_without_a_description_or_type_is_labelled_from_the_prompt() {
        let long = "p".repeat(200);
        let parsed = StreamParser::default().line(
            &serde_json::json!({"type":"assistant","message":{"id":"m","content":[{"type":"tool_use","id":"t","name":"Task","input":{"prompt":long}}]}})
                .to_string(),
        );
        let [Parsed::StepStarted(step)] = &parsed[..] else {
            panic!("{parsed:?}");
        };
        assert_eq!(step.title, "p".repeat(SUBAGENT_LABEL_LIMIT));
        assert!(step.input.starts_with(&format!(
            r#"{{"subagent_type":"general-purpose","description":"{}","prompt":"#,
            "p".repeat(SUBAGENT_LABEL_LIMIT)
        )));
    }

    #[test]
    fn a_nested_subagent_nests_under_the_subagent_that_started_it() {
        let mut parser = StreamParser::default();
        let parsed = parse_values(
            &mut parser,
            &[
                serde_json::json!({"type":"assistant","message":{"id":"m1","content":[{"type":"tool_use","id":"outer","name":"Agent","input":{"description":"outer"}}]}}),
                serde_json::json!({"type":"assistant","parent_tool_use_id":"outer","message":{"id":"m2","content":[{"type":"tool_use","id":"middle","name":"Task","input":{"description":"middle"}}]}}),
                serde_json::json!({"type":"assistant","parent_tool_use_id":"middle","message":{"id":"m3","content":[{"type":"tool_use","id":"leaf","name":"Read","input":{"file_path":"/a"}}]}}),
            ],
        );
        let started = started_steps(&parsed);
        assert_eq!(started.len(), 3);
        assert_eq!(
            started[1].parent_step_id.as_deref(),
            Some(started[0].id.as_str())
        );
        assert_eq!(
            started[2].parent_step_id.as_deref(),
            Some(started[1].id.as_str())
        );
    }

    #[test]
    fn inner_events_of_an_unknown_subagent_are_ignored_and_a_finished_one_still_adopts_steps() {
        let mut parser = StreamParser::default();
        let orphan = serde_json::json!({"type":"assistant","parent_tool_use_id":"nope","message":{"id":"m","content":[{"type":"tool_use","id":"x","name":"Bash","input":{"command":"ls"}}]}});
        assert!(parser.line(&orphan.to_string()).is_empty());
        let mut stream = subagent_stream();
        let late = serde_json::json!({"type":"assistant","parent_tool_use_id":"toolu_agent","message":{"id":"m9","content":[{"type":"tool_use","id":"late","name":"Bash","input":{"command":"pwd"}}]}});
        stream.push(late);
        let parsed = parse_values(&mut parser, &stream);
        let started = started_steps(&parsed);
        assert_eq!(started.len(), 3);
        assert_eq!(
            started[2].parent_step_id.as_deref(),
            Some(started[0].id.as_str())
        );
    }

    #[test]
    fn a_key_in_a_subagent_prompt_and_in_inner_steps_is_redacted() {
        let mut parser = StreamParser::new(Redactor::for_key(Some(&SecretString::new(
            "sk-ant-api03-SECRETSECRETSECRET",
        ))));
        let parsed = parse_values(
            &mut parser,
            &[
                serde_json::json!({"type":"assistant","message":{"id":"m1","content":[{"type":"tool_use","id":"a","name":"Agent","input":{"description":"use sk-ant-api03-SECRETSECRETSECRET","prompt":"key sk-ant-api03-SECRETSECRETSECRET"}}]}}),
                serde_json::json!({"type":"assistant","parent_tool_use_id":"a","message":{"id":"m2","content":[{"type":"tool_use","id":"b","name":"Bash","input":{"command":"echo sk-ant-api03-SECRETSECRETSECRET"}}]}}),
                serde_json::json!({"type":"user","parent_tool_use_id":"a","message":{"content":[{"type":"tool_result","tool_use_id":"b","content":"out sk-ant-api03-SECRETSECRETSECRET","is_error":false}]}}),
            ],
        );
        assert!(!format!("{parsed:?}").contains("SECRETSECRET"));
        assert!(format!("{parsed:?}").contains("[redacted]"));
    }

    #[test]
    fn a_subagent_launched_in_the_background_reports_a_launch_and_keeps_its_children() {
        let mut parser = StreamParser::default();
        let parsed = parse_values(
            &mut parser,
            &[
                serde_json::json!({"type":"assistant","message":{"id":"m1","content":[{"type":"tool_use","id":"bg","name":"Agent","input":{"description":"later","run_in_background":true}}]}}),
                serde_json::json!({"type":"user","tool_use_result":{"status":"async_launched","agentId":"x"},"message":{"content":[{"type":"tool_result","tool_use_id":"bg","content":[{"type":"text","text":"Async agent launched successfully.\nagentId: x"}]}]}}),
                serde_json::json!({"type":"assistant","parent_tool_use_id":"bg","message":{"id":"m2","content":[{"type":"tool_use","id":"in","name":"Bash","input":{"command":"ls"}}]}}),
            ],
        );
        let started = started_steps(&parsed);
        assert_eq!(started.len(), 2);
        assert_eq!(
            started[1].parent_step_id.as_deref(),
            Some(started[0].id.as_str())
        );
        assert!(ended_steps(&parsed).is_empty());
        assert!(matches!(
            &parsed[1],
            Parsed::StepBackground { step_id, output }
                if step_id == &started[0].id && output.contains("Async agent launched")
        ));
    }

    #[test]
    fn a_launch_marker_on_an_ordinary_tool_or_a_failed_subagent_ends_the_step() {
        let mut parser = StreamParser::default();
        let parsed = parse_values(
            &mut parser,
            &[
                serde_json::json!({"type":"assistant","message":{"id":"m1","content":[{"type":"tool_use","id":"b","name":"Bash","input":{"command":"echo"}},{"type":"tool_use","id":"a","name":"Agent","input":{"description":"x"}}]}}),
                serde_json::json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"b","content":"async_launched","is_error":false},{"type":"tool_result","tool_use_id":"a","content":"async_launched","is_error":true}]}}),
            ],
        );
        assert_eq!(ended_steps(&parsed).len(), 2);
    }

    #[test]
    fn the_saved_subagent_input_stays_valid_json_and_splits_into_task_and_result() {
        let long = "p\"\n".repeat(5000);
        let json = subagent_input("Explore", "look", &serde_json::json!({"prompt": long}));
        assert!(json.len() <= DETAIL_LIMIT);
        let value: Value = serde_json::from_str(&json).unwrap();
        assert!(value["prompt"].as_str().unwrap().ends_with('…'));
        let (task, result) = subagent_parts(&format!("{json}\n\nfound it"));
        assert!(task.unwrap().starts_with("p\"\n"));
        assert_eq!(result.as_deref(), Some("found it"));
        assert_eq!(subagent_parts(&json).1, None);
        assert_eq!(subagent_parts("not json"), (None, None));
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
                text: "Boom".to_owned(),
                api_error_status: None
            }]
        );
    }

    #[test]
    fn an_error_result_carries_the_api_error_status() {
        let line = r#"{"type":"result","subtype":"success","is_error":true,"api_error_status":401,"result":"Invalid API key"}"#;
        assert_eq!(
            StreamParser::default().line(line),
            vec![Parsed::Result {
                is_error: true,
                text: "Invalid API key".to_owned(),
                api_error_status: Some(401)
            }]
        );
    }

    #[test]
    fn a_compact_boundary_event_is_reported_and_other_system_events_are_not() {
        let mut parser = StreamParser::default();
        let boundary = r#"{"type":"system","subtype":"compact_boundary","session_id":"s","compact_metadata":{"trigger":"auto","pre_tokens":1000}}"#;
        assert_eq!(parser.line(boundary), vec![Parsed::Compacted]);
        let micro = r#"{"type":"system","subtype":"microcompact_boundary"}"#;
        assert!(parser.line(micro).is_empty());
    }

    #[test]
    fn sign_in_failures_are_told_from_other_errors() {
        for text in [
            "API Error: 401 {\"type\":\"error\",\"error\":{\"type\":\"authentication_error\"}}",
            "Invalid X-API-Key",
            "Invalid API key · Fix external API key",
            "Not logged in · Please run /login",
            "Please run /login",
            "OAuth token has expired. Please obtain a new token",
            "AUTHENTICATION_ERROR",
        ] {
            assert!(is_sign_in_failure(text, None), "{text}");
        }
        assert!(is_sign_in_failure("anything", Some(401)));
        assert!(is_sign_in_failure("", Some(401)));
        for (text, status) in [
            ("Credit balance is too low", None),
            ("Credit balance is too low", Some(400)),
            ("Boom", None),
            ("", None),
            ("rate limited", Some(429)),
        ] {
            assert!(!is_sign_in_failure(text, status), "{text}");
        }
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
