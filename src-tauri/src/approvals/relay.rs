use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use serde_json::{json, Value};
use tokio::io::{AsyncBufRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::unix::OwnedWriteHalf;
use tokio::net::UnixStream;
use tokio::sync::{mpsc, watch, Mutex};

use super::session::read_bounded_line;
use super::{DENY_UNREACHABLE, SERVER_NAME, TOOL_NAME};

const PROTOCOL_VERSION: &str = "2025-11-25";
const TOOL_DESCRIPTION: &str = "Decides one permission request for Claude Code.";
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;

struct State {
    pending: HashMap<u64, Value>,
    next: u64,
    app: Option<OwnedWriteHalf>,
}

type Shared = Arc<Mutex<State>>;

fn result_of(id: &Value, result: Value) -> String {
    json!({"jsonrpc": "2.0", "id": id, "result": result}).to_string()
}

fn error_of(id: &Value, code: i64, message: &str) -> String {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}).to_string()
}

fn decision_result(id: &Value, decision: &Value) -> String {
    result_of(
        id,
        json!({"content": [{"type": "text", "text": decision.to_string()}]}),
    )
}

fn denied(message: &str) -> Value {
    json!({"behavior": "deny", "message": message})
}

fn tool_definition() -> Value {
    json!({
        "name": TOOL_NAME,
        "description": TOOL_DESCRIPTION,
        "inputSchema": {
            "type": "object",
            "properties": {
                "tool_name": {"type": "string"},
                "input": {"type": "object"},
                "tool_use_id": {"type": "string"}
            },
            "required": ["tool_name", "input"]
        }
    })
}

async fn forward(
    shared: &Shared,
    pending_count: &watch::Sender<usize>,
    id: &Value,
    arguments: &Value,
) -> Option<String> {
    let (Some(tool), Some(input)) = (
        arguments.get("tool_name").and_then(Value::as_str),
        arguments.get("input").filter(|input| input.is_object()),
    ) else {
        return Some(decision_result(
            id,
            &denied("The request could not be read, so it was denied."),
        ));
    };
    let mut state = shared.lock().await;
    state.next += 1;
    let number = state.next;
    let mut request = json!({"id": number, "tool": tool, "input": input});
    if let Some(use_id) = arguments.get("tool_use_id").and_then(Value::as_str) {
        request["toolUseId"] = Value::String(use_id.to_owned());
    }
    let mut line = request.to_string();
    line.push('\n');
    let sent = match state.app.as_mut() {
        Some(app) => app.write_all(line.as_bytes()).await.is_ok(),
        None => false,
    };
    if !sent {
        state.app = None;
        return Some(decision_result(id, &denied(DENY_UNREACHABLE)));
    }
    state.pending.insert(number, id.clone());
    pending_count.send_replace(state.pending.len());
    None
}

async fn handle_line(
    line: &str,
    shared: &Shared,
    pending_count: &watch::Sender<usize>,
    out: &mpsc::UnboundedSender<String>,
) {
    let Ok(message) = serde_json::from_str::<Value>(line) else {
        return;
    };
    let (Some(method), Some(id)) = (
        message.get("method").and_then(Value::as_str),
        message.get("id"),
    ) else {
        return;
    };
    let params = message.get("params").cloned().unwrap_or(Value::Null);
    let reply = match method {
        "initialize" => {
            let version = params
                .get("protocolVersion")
                .and_then(Value::as_str)
                .unwrap_or(PROTOCOL_VERSION);
            Some(result_of(
                id,
                json!({
                    "protocolVersion": version,
                    "capabilities": {"tools": {}},
                    "serverInfo": {"name": SERVER_NAME, "version": env!("CARGO_PKG_VERSION")}
                }),
            ))
        }
        "ping" => Some(result_of(id, json!({}))),
        "tools/list" => Some(result_of(id, json!({"tools": [tool_definition()]}))),
        "tools/call" => {
            if params.get("name").and_then(Value::as_str) == Some(TOOL_NAME) {
                let arguments = params.get("arguments").cloned().unwrap_or(Value::Null);
                forward(shared, pending_count, id, &arguments).await
            } else {
                Some(error_of(id, INVALID_PARAMS, "Unknown tool."))
            }
        }
        _ => Some(error_of(id, METHOD_NOT_FOUND, "Method not found.")),
    };
    if let Some(reply) = reply {
        let _ = out.send(reply);
    }
}

async fn read_app(
    mut reader: impl AsyncBufRead + Unpin,
    shared: Shared,
    pending_count: Arc<watch::Sender<usize>>,
    out: mpsc::UnboundedSender<String>,
) {
    let mut buffer = Vec::new();
    while let Ok(Some(line)) = read_bounded_line(&mut reader, &mut buffer).await {
        let Ok(Value::Object(mut fields)) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let Some(number) = fields.remove("id").and_then(|id| id.as_u64()) else {
            continue;
        };
        let mut state = shared.lock().await;
        if let Some(id) = state.pending.remove(&number) {
            let _ = out.send(decision_result(&id, &Value::Object(fields)));
            pending_count.send_replace(state.pending.len());
        }
    }
    let mut state = shared.lock().await;
    state.app = None;
    for (_, id) in state.pending.drain() {
        let _ = out.send(decision_result(&id, &denied(DENY_UNREACHABLE)));
    }
    pending_count.send_replace(0);
}

pub async fn serve(
    app: Option<UnixStream>,
    token: &str,
    input: impl AsyncBufRead + Unpin,
    output: impl AsyncWrite + Send + Unpin + 'static,
) {
    let (out, mut lines) = mpsc::unbounded_channel::<String>();
    let writer = tokio::spawn(async move {
        let mut output = output;
        while let Some(mut line) = lines.recv().await {
            line.push('\n');
            if output.write_all(line.as_bytes()).await.is_err() || output.flush().await.is_err() {
                break;
            }
        }
    });
    let (pending_count, mut drained) = watch::channel(0usize);
    let pending_count = Arc::new(pending_count);
    let mut app_writer = None;
    let mut app_reader = None;
    if let Some(stream) = app {
        let (read, mut write) = stream.into_split();
        let hello = format!("{}\n", json!({"hello": token}));
        if write.write_all(hello.as_bytes()).await.is_ok() {
            app_writer = Some(write);
            app_reader = Some(BufReader::new(read));
        }
    }
    let shared: Shared = Arc::new(Mutex::new(State {
        pending: HashMap::new(),
        next: 0,
        app: app_writer,
    }));
    let reader_task = app_reader.map(|reader| {
        tokio::spawn(read_app(
            reader,
            Arc::clone(&shared),
            Arc::clone(&pending_count),
            out.clone(),
        ))
    });
    let mut input = input;
    let mut buffer = Vec::new();
    while let Ok(Some(line)) = read_bounded_line(&mut input, &mut buffer).await {
        handle_line(&line, &shared, &pending_count, &out).await;
    }
    let _ = drained.wait_for(|count| *count == 0).await;
    if let Some(task) = reader_task {
        task.abort();
    }
    drop(out);
    let _ = writer.await;
}

pub fn run(socket: &Path, token: &str) -> i32 {
    let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        return 1;
    };
    runtime.block_on(async {
        let stream = UnixStream::connect(socket).await.ok();
        serve(
            stream,
            token,
            BufReader::new(tokio::io::stdin()),
            tokio::io::stdout(),
        )
        .await;
        0
    })
}
