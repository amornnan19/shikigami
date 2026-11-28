use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{duplex, AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream};
use tokio::net::UnixStream;
use tokio::time::timeout;

use super::*;

const WAIT: Duration = Duration::from_secs(5);

struct Client {
    input: DuplexStream,
    output: BufReader<DuplexStream>,
}

impl Client {
    async fn send(&mut self, message: Value) {
        let mut line = message.to_string();
        line.push('\n');
        self.input.write_all(line.as_bytes()).await.unwrap();
    }

    async fn receive(&mut self) -> Value {
        let mut line = String::new();
        timeout(WAIT, self.output.read_line(&mut line))
            .await
            .expect("the relay answers")
            .unwrap();
        serde_json::from_str(&line).unwrap()
    }

    async fn call(&mut self, id: u64, tool: &str, input: Value) {
        self.send(json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": {
                "name": TOOL_NAME,
                "arguments": {"tool_name": tool, "input": input, "tool_use_id": format!("toolu_{id}")}
            }
        }))
        .await;
    }

    async fn decision(&mut self, id: u64) -> Value {
        let reply = self.receive().await;
        assert_eq!(reply["id"], json!(id));
        let text = reply["result"]["content"][0]["text"].as_str().unwrap();
        assert!(reply["result"].get("structuredContent").is_none());
        serde_json::from_str(text).unwrap()
    }
}

fn relay(app: Option<UnixStream>, token: &str) -> (Client, tokio::task::JoinHandle<()>) {
    let (input, relay_input) = duplex(1 << 20);
    let (relay_output, output) = duplex(1 << 20);
    let token = token.to_owned();
    let task = tokio::spawn(async move {
        serve_relay(app, &token, BufReader::new(relay_input), relay_output).await;
    });
    (
        Client {
            input,
            output: BufReader::new(output),
        },
        task,
    )
}

async fn connected(dir: &std::path::Path) -> (Channel, UnixStream) {
    let channel = Channel::bind(dir).unwrap();
    let stream = UnixStream::connect(channel.path()).await.unwrap();
    (channel, stream)
}

async fn next(channel: &mut Channel) -> Inbound {
    timeout(WAIT, channel.next())
        .await
        .expect("a request arrives")
        .unwrap()
}

#[tokio::test]
async fn the_relay_speaks_the_mcp_handshake_and_offers_one_tool() {
    let (mut client, _task) = relay(None, "t");
    client.send(json!({"jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {"protocolVersion": "2025-11-25", "capabilities": {}}})).await;
    let reply = client.receive().await;
    assert_eq!(reply["id"], json!(0));
    assert_eq!(reply["result"]["protocolVersion"], "2025-11-25");
    assert_eq!(reply["result"]["serverInfo"]["name"], "shikigami");
    assert_eq!(reply["result"]["capabilities"], json!({"tools": {}}));
    client
        .send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))
        .await;
    client
        .send(json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}))
        .await;
    let reply = client.receive().await;
    assert_eq!(reply["id"], json!(1));
    let tools = reply["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0]["name"], "approve");
    assert!(tools[0].get("outputSchema").is_none());
    client
        .send(json!({"jsonrpc": "2.0", "id": 2, "method": "server/discover"}))
        .await;
    assert_eq!(client.receive().await["error"]["code"], -32601);
    client
        .send(json!({"jsonrpc": "2.0", "id": 3, "method": "ping"}))
        .await;
    assert_eq!(client.receive().await["result"], json!({}));
    client
        .send(json!({"jsonrpc": "2.0", "id": 5, "method": "tools/call", "params": {"name": "other", "arguments": {}}}))
        .await;
    let reply = client.receive().await;
    assert_eq!(reply["id"], json!(5));
    assert_eq!(reply["error"]["code"], -32602);
}

#[tokio::test]
async fn a_request_goes_to_the_app_and_its_allow_comes_back_with_the_original_input() {
    let dir = tempfile::tempdir().unwrap();
    let (mut channel, stream) = connected(dir.path()).await;
    let (mut client, _task) = relay(Some(stream), channel.token());
    let input = json!({"command": "ls -la", "description": "List files"});
    client.call(7, "Bash", input.clone()).await;
    let Inbound::Request {
        conn,
        id,
        tool,
        input: received,
        tool_use_id,
    } = next(&mut channel).await
    else {
        panic!("a request was expected");
    };
    assert_eq!(tool, "Bash");
    assert_eq!(received, input);
    assert_eq!(tool_use_id.as_deref(), Some("toolu_7"));
    assert!(channel.reply(conn, id, allow_reply(&received)).await);
    assert_eq!(
        client.decision(7).await,
        json!({"behavior": "allow", "updatedInput": input})
    );
}

#[tokio::test]
async fn a_deny_from_the_app_comes_back_with_its_message() {
    let dir = tempfile::tempdir().unwrap();
    let (mut channel, stream) = connected(dir.path()).await;
    let (mut client, _task) = relay(Some(stream), channel.token());
    client
        .call(1, "WebFetch", json!({"url": "https://example.com"}))
        .await;
    let Inbound::Request { conn, id, .. } = next(&mut channel).await else {
        panic!("a request was expected");
    };
    channel.reply(conn, id, deny_reply(DENY_USER)).await;
    assert_eq!(
        client.decision(1).await,
        json!({"behavior": "deny", "message": "The user denied this request."})
    );
}

#[tokio::test]
async fn two_requests_in_flight_are_answered_independently_and_out_of_order() {
    let dir = tempfile::tempdir().unwrap();
    let (mut channel, stream) = connected(dir.path()).await;
    let (mut client, _task) = relay(Some(stream), channel.token());
    client.call(1, "Bash", json!({"command": "one"})).await;
    client.call(2, "Bash", json!({"command": "two"})).await;
    let Inbound::Request {
        conn, id: first, ..
    } = next(&mut channel).await
    else {
        panic!("a request was expected");
    };
    let Inbound::Request { id: second, .. } = next(&mut channel).await else {
        panic!("a request was expected");
    };
    channel.reply(conn, second, deny_reply(DENY_USER)).await;
    assert_eq!(client.decision(2).await["behavior"], "deny");
    channel
        .reply(conn, first, allow_reply(&json!({"command": "one"})))
        .await;
    assert_eq!(client.decision(1).await["behavior"], "allow");
}

#[tokio::test]
async fn the_relay_never_allows_by_itself_when_the_app_is_missing_or_gone() {
    let (mut client, _task) = relay(None, "t");
    client.call(1, "Bash", json!({"command": "ls"})).await;
    assert_eq!(
        client.decision(1).await,
        json!({"behavior": "deny", "message": DENY_UNREACHABLE})
    );

    let dir = tempfile::tempdir().unwrap();
    let (mut channel, stream) = connected(dir.path()).await;
    let (mut client, _task) = relay(Some(stream), channel.token());
    client.call(2, "Bash", json!({"command": "ls"})).await;
    assert!(matches!(next(&mut channel).await, Inbound::Request { .. }));
    drop(channel);
    assert_eq!(
        client.decision(2).await,
        json!({"behavior": "deny", "message": DENY_UNREACHABLE})
    );
    client.call(3, "Bash", json!({"command": "ls"})).await;
    assert_eq!(client.decision(3).await["behavior"], "deny");
}

#[tokio::test]
async fn a_relay_with_the_wrong_token_reaches_nobody_and_is_denied() {
    let dir = tempfile::tempdir().unwrap();
    let (mut channel, stream) = connected(dir.path()).await;
    let (mut client, _task) = relay(Some(stream), "not-the-token");
    client.call(1, "Bash", json!({"command": "ls"})).await;
    assert_eq!(client.decision(1).await["behavior"], "deny");
    assert!(timeout(Duration::from_millis(300), channel.next())
        .await
        .is_err());
}

#[tokio::test]
async fn an_unreadable_request_is_denied_and_never_forwarded() {
    let dir = tempfile::tempdir().unwrap();
    let (mut channel, stream) = connected(dir.path()).await;
    let (mut client, _task) = relay(Some(stream), channel.token());
    client
        .send(json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "approve", "arguments": {"tool_name": "Bash"}}}))
        .await;
    assert_eq!(client.decision(1).await["behavior"], "deny");
    assert!(timeout(Duration::from_millis(300), channel.next())
        .await
        .is_err());
}

#[tokio::test]
async fn the_relay_waits_for_open_requests_after_its_input_closes() {
    let dir = tempfile::tempdir().unwrap();
    let (mut channel, stream) = connected(dir.path()).await;
    let (mut client, task) = relay(Some(stream), channel.token());
    client.call(1, "Bash", json!({"command": "ls"})).await;
    let Inbound::Request { conn, id, .. } = next(&mut channel).await else {
        panic!("a request was expected");
    };
    client.input.shutdown().await.unwrap();
    assert!(timeout(Duration::from_millis(300), async {
        let _ = (&mut client.output).lines().next_line().await;
    })
    .await
    .is_err());
    channel.reply(conn, id, deny_reply(DENY_USER)).await;
    assert_eq!(client.decision(1).await["behavior"], "deny");
    timeout(WAIT, task).await.unwrap().unwrap();
}

#[tokio::test]
async fn a_closed_connection_is_reported_and_the_socket_file_is_removed_with_the_channel() {
    let dir = tempfile::tempdir().unwrap();
    let (mut channel, stream) = connected(dir.path()).await;
    let path = channel.path().to_path_buf();
    let mut hello = stream;
    hello
        .write_all(format!("{}\n", json!({"hello": channel.token()})).as_bytes())
        .await
        .unwrap();
    hello
        .write_all(b"{\"id\":1,\"tool\":\"Bash\",\"input\":{\"command\":\"ls\"}}\n")
        .await
        .unwrap();
    assert!(matches!(next(&mut channel).await, Inbound::Request { .. }));
    drop(hello);
    assert!(matches!(next(&mut channel).await, Inbound::Closed { .. }));
    assert!(path.exists());
    drop(channel);
    assert!(!path.exists());
}

#[tokio::test]
async fn the_socket_is_private_to_the_user() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let channel = Channel::bind(dir.path()).unwrap();
    let mode = std::fs::metadata(channel.path())
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600);
    assert_eq!(channel.path().file_name().unwrap().len(), 13);
}

#[test]
fn an_allow_leaves_out_an_input_too_large_to_send_back() {
    let small = json!({"command": "ls"});
    assert_eq!(
        allow_reply(&small),
        json!({"behavior": "allow", "updatedInput": small})
    );
    let large = json!({"command": "x".repeat(40 * 1024)});
    assert_eq!(allow_reply(&large), json!({"behavior": "allow"}));
}

#[test]
fn the_relay_starts_only_for_its_own_flag_and_needs_both_arguments() {
    let args = |list: &[&str]| list.iter().map(OsString::from).collect::<Vec<_>>();
    assert_eq!(run_relay(args(&["shikigami"])), None);
    assert_eq!(run_relay(args(&["shikigami", "--other", "a", "b"])), None);
    assert_eq!(run_relay(args(&["shikigami", "--approval-relay"])), Some(2));
    assert_eq!(
        run_relay(args(&["shikigami", "--approval-relay", "/tmp/x"])),
        Some(2)
    );
}

#[test]
fn the_config_names_the_relay_command_with_the_socket_the_token_and_a_long_call_limit() {
    let config: Value = serde_json::from_str(&relay_config(
        std::path::Path::new("/Applications/Shikigami.app/Contents/MacOS/shikigami"),
        std::path::Path::new("/data/tmp/ap-1.s"),
        "tok",
    ))
    .unwrap();
    let server = &config["mcpServers"]["shikigami"];
    assert_eq!(
        server["command"],
        "/Applications/Shikigami.app/Contents/MacOS/shikigami"
    );
    assert_eq!(
        server["args"],
        json!(["--approval-relay", "/data/tmp/ap-1.s", "tok"])
    );
    assert!(server["timeout"].as_u64().unwrap() > 600_000);
    assert_eq!(PROMPT_TOOL, "mcp__shikigami__approve");
}

#[tokio::test]
async fn a_folder_too_deep_for_a_socket_path_gets_its_socket_in_a_private_short_folder() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let deep = dir.path().join("x".repeat(110));
    let channel = Channel::bind(&deep).unwrap();
    let path = channel.path().to_path_buf();
    assert!(path.as_os_str().len() <= 100, "{}", path.display());
    assert!(!path.starts_with(&deep));
    assert_eq!(path.parent().unwrap(), socket_dir(&deep));
    assert_eq!(socket_dir(&deep), socket_dir(&deep));
    assert_ne!(
        socket_dir(&deep),
        socket_dir(&dir.path().join("y".repeat(110)))
    );
    let folder = std::fs::metadata(path.parent().unwrap()).unwrap();
    assert_eq!(folder.permissions().mode() & 0o777, 0o700);
    assert!(UnixStream::connect(&path).await.is_ok());
    drop(channel);
    assert!(!path.exists());
    let _ = std::fs::remove_dir(socket_dir(&deep));
}

#[test]
fn a_short_folder_keeps_its_socket_where_it_is() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(socket_dir(dir.path()), dir.path());
}
