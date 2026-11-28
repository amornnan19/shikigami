use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::UnixStream;
use tokio::time::timeout;

use super::*;
use crate::approvals::store as approval_store;
use crate::approvals::{self, ApprovalView};

const WAIT: Duration = Duration::from_secs(10);
const LEAKY_TOKEN: &str = "sk-ant-api03-LEAKYLEAKYLEAKYLEAKYLEAKYLEAKY";

fn init_line() -> String {
    stream_lines(&[
        serde_json::json!({"type":"system","subtype":"init","session_id":"s1","tools":["Read","Edit","Write","Bash"]}),
    ])
}

fn install_waiting(world: &World) {
    let init = world.fixture("init.jsonl", &init_line());
    let result = world.fixture(
        "result.jsonl",
        &stream_lines(&[serde_json::json!({"type":"result","is_error":false,"result":"ok"})]),
    );
    let done = world.records().join("done");
    world.install(&recording(
        world,
        &format!(
            "cat \"{}\"\nwhile [ ! -e \"{}\" ]; do sleep 0.05; done\ncat \"{}\"",
            init.display(),
            done.display(),
            result.display()
        ),
    ));
}

fn install_stubborn(world: &World) {
    let init = world.fixture("init.jsonl", &init_line());
    world.install(&recording(
        world,
        &format!(
            "trap '' TERM\ntouch \"{}/ready\"\ncat \"{}\"\nwhile true; do sleep 0.05; done",
            world.records().display(),
            init.display()
        ),
    ));
}

async fn wait_until_stubborn(world: &World) {
    timeout(WAIT, async {
        while !world.records().join("ready").exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the CLI ignores the term signal");
}

fn finish_cli(world: &World) {
    fs::write(world.records().join("done"), "").unwrap();
}

struct Relay {
    reader: BufReader<OwnedReadHalf>,
    writer: OwnedWriteHalf,
}

impl Relay {
    async fn connect(world: &World) -> Self {
        let config = timeout(WAIT, async {
            loop {
                if let Ok(text) = fs::read_to_string(world.records().join("args.0")) {
                    let lines: Vec<&str> = text.lines().collect();
                    let at = lines.iter().position(|arg| *arg == "--mcp-config");
                    if let Some(path) = at.and_then(|at| lines.get(at + 1)) {
                        if let Ok(config) = fs::read_to_string(path) {
                            return config;
                        }
                    }
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("the CLI is started with an MCP config");
        let config: Value = serde_json::from_str(&config).unwrap();
        let server = &config["mcpServers"]["shikigami"];
        let socket = server["args"][1].as_str().unwrap();
        let token = server["args"][2].as_str().unwrap();
        let stream = UnixStream::connect(socket).await.unwrap();
        let (read, mut writer) = stream.into_split();
        writer
            .write_all(format!("{}\n", json!({"hello": token})).as_bytes())
            .await
            .unwrap();
        Self {
            reader: BufReader::new(read),
            writer,
        }
    }

    async fn request(&mut self, id: u64, tool: &str, input: Value) {
        let line =
            json!({"id": id, "tool": tool, "input": input, "toolUseId": format!("toolu_{id}")});
        self.writer
            .write_all(format!("{line}\n").as_bytes())
            .await
            .unwrap();
    }

    async fn reply(&mut self) -> Value {
        let mut line = String::new();
        timeout(WAIT, self.reader.read_line(&mut line))
            .await
            .expect("an answer arrives")
            .unwrap();
        serde_json::from_str(&line).unwrap()
    }
}

fn rows(world: &World) -> Vec<ApprovalView> {
    let turn = world.turns().last().map(|turn| turn.id.clone());
    match turn {
        Some(turn) => world
            .db
            .with_connection(|c| approval_store::for_turn(c, &turn))
            .unwrap(),
        None => Vec::new(),
    }
}

async fn wait_for_rows(world: &World, count: usize) -> Vec<ApprovalView> {
    timeout(WAIT, async {
        loop {
            let found = rows(world);
            if found.len() >= count {
                return found;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the cards appear")
}

async fn wait_for_status(world: &World, id: &str, status: &str) -> ApprovalView {
    timeout(WAIT, async {
        loop {
            if let Some(row) = rows(world).into_iter().find(|row| row.id == id) {
                if row.status == status {
                    return row;
                }
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("the card reaches {status}"))
}

fn answer(world: &World, id: &str, allow: bool) -> Result<(), approvals::ApprovalError> {
    approvals::answer(&world.db, &world.registry, id, allow)
}

fn kinds_of(world: &World, wanted: &str) -> usize {
    world
        .sink
        .kinds()
        .iter()
        .filter(|kind| kind.as_str() == wanted)
        .count()
}

async fn drive<F, Fut>(world: &World, launch: Launch, driver: F) -> RunEnd
where
    F: FnOnce(String) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let turn_id = launch.turn_id.clone();
    assert!(world.registry.begin("a", &turn_id));
    let (end, ()) = timeout(Duration::from_secs(30), async {
        tokio::join!(
            run(
                &world.db,
                &world.sink,
                &world.registry,
                launch,
                ApiKey::Subscription
            ),
            driver(turn_id)
        )
    })
    .await
    .expect("the turn ends");
    end
}

#[tokio::test]
async fn a_request_becomes_a_pending_card_and_allow_once_replies_with_the_original_input() {
    let world = World::new();
    install_waiting(&world);
    let launch = world.launch("go");
    let input = json!({"command": "ls -la", "description": "List the folder"});
    let end = drive(&world, launch, |_| async {
        let mut relay = Relay::connect(&world).await;
        relay.request(1, "Bash", input.clone()).await;
        let pending = wait_for_rows(&world, 1).await;
        let card = &pending[0];
        assert_eq!(card.status, "pending");
        assert_eq!(card.tool, "Bash");
        assert_eq!(card.summary, "ls -la");
        assert_eq!(card.reason.as_deref(), Some("List the folder"));
        assert_eq!(card.folder, world.workspace.path().to_string_lossy());
        assert_eq!(card.answered_by, None);
        assert_eq!(kinds_of(&world, "approval-requested"), 1);
        answer(&world, &card.id, true).unwrap();
        let reply = relay.reply().await;
        assert_eq!(reply["id"], 1);
        assert_eq!(reply["behavior"], "allow");
        assert_eq!(reply["updatedInput"], input);
        let row = wait_for_status(&world, &card.id, "allowed").await;
        assert_eq!(row.answered_by.as_deref(), Some("user"));
        assert!(row.answered_at.is_some());
        assert_eq!(kinds_of(&world, "approval-resolved"), 1);
        finish_cli(&world);
    })
    .await;
    assert_eq!(end, RunEnd::Finished);
    let turn = only_turn(&world);
    assert_eq!(turn.status, "completed");
    assert!(turn
        .items
        .iter()
        .any(|item| matches!(item, ItemView::Approval(card) if card.status == "allowed")));
    assert_eq!(
        fs::read_dir(world.data.path().join("tmp")).unwrap().count(),
        0
    );
    let args = args(&world, 0);
    let at = args
        .iter()
        .position(|arg| arg == "--permission-prompt-tool");
    assert_eq!(args[at.unwrap() + 1], "mcp__shikigami__approve");
    assert!(args.iter().any(|arg| arg == "--strict-mcp-config"));
    assert!(world.registry.signals().is_empty());
}

#[tokio::test]
async fn deny_replies_that_the_user_denied_the_request_and_the_turn_goes_on() {
    let world = World::new();
    install_waiting(&world);
    let launch = world.launch("go");
    let end = drive(&world, launch, |_| async {
        let mut relay = Relay::connect(&world).await;
        relay
            .request(1, "Bash", json!({"command": "rm -rf x"}))
            .await;
        let card = wait_for_rows(&world, 1).await.remove(0);
        answer(&world, &card.id, false).unwrap();
        let reply = relay.reply().await;
        assert_eq!(reply["behavior"], "deny");
        assert_eq!(reply["message"], "The user denied this request.");
        assert!(reply.get("updatedInput").is_none());
        let row = wait_for_status(&world, &card.id, "denied").await;
        assert_eq!(row.answered_by.as_deref(), Some("user"));
        finish_cli(&world);
    })
    .await;
    assert_eq!(end, RunEnd::Finished);
    assert_eq!(only_turn(&world).status, "completed");
}

#[tokio::test]
async fn an_unanswered_card_times_out_as_denied() {
    let world = World::new();
    install_waiting(&world);
    let mut launch = world.launch("go");
    launch.approval_timeout = Duration::from_millis(300);
    let end = drive(&world, launch, |_| async {
        let mut relay = Relay::connect(&world).await;
        relay.request(1, "Bash", json!({"command": "ls"})).await;
        let card = wait_for_rows(&world, 1).await.remove(0);
        assert_eq!(card.status, "pending");
        let reply = relay.reply().await;
        assert_eq!(reply["behavior"], "deny");
        assert_eq!(
            reply["message"],
            "No answer within 10 minutes, so this request was denied."
        );
        let row = wait_for_status(&world, &card.id, "timed-out").await;
        assert_eq!(row.answered_by.as_deref(), Some("timeout"));
        assert_eq!(
            answer(&world, &card.id, true).unwrap_err().code,
            "already-answered"
        );
        finish_cli(&world);
    })
    .await;
    assert_eq!(end, RunEnd::Finished);
}

#[tokio::test]
async fn a_stop_denies_every_open_card_and_tells_the_relay() {
    let world = World::new();
    install_waiting(&world);
    let launch = world.launch("go");
    let world_ref = &world;
    let end = drive(&world, launch, |turn_id| async move {
        let world = world_ref;
        let mut relay = Relay::connect(world).await;
        relay.request(1, "Bash", json!({"command": "one"})).await;
        relay
            .request(2, "WebFetch", json!({"url": "https://example.com"}))
            .await;
        let cards = wait_for_rows(world, 2).await;
        world.registry.request_stop("a", &turn_id).unwrap();
        for _ in 0..2 {
            let reply = relay.reply().await;
            assert_eq!(reply["behavior"], "deny");
            assert_eq!(
                reply["message"],
                "The turn stopped, so this request was denied."
            );
        }
        for card in cards {
            let row = wait_for_status(world, &card.id, "denied").await;
            assert_eq!(row.answered_by.as_deref(), Some("stop"));
        }
    })
    .await;
    assert_eq!(end, RunEnd::Finished);
    assert_eq!(only_turn(&world).status, "stopped");
}

#[tokio::test]
async fn shutting_down_denies_every_open_card_as_shutdown_and_leaves_the_turn_running() {
    let world = World::new();
    install_waiting(&world);
    let launch = world.launch("go");
    let end = drive(&world, launch, |_| async {
        let mut relay = Relay::connect(&world).await;
        relay.request(1, "Bash", json!({"command": "one"})).await;
        let card = wait_for_rows(&world, 1).await.remove(0);
        world.registry.kill_all(Duration::from_secs(2));
        let row = wait_for_status(&world, &card.id, "denied").await;
        assert_eq!(row.answered_by.as_deref(), Some("shutdown"));
        let reply = relay.reply().await;
        assert_eq!(reply["behavior"], "deny");
    })
    .await;
    assert_eq!(end, RunEnd::Closing);
    assert_eq!(only_turn(&world).status, "running");
}

#[tokio::test]
async fn a_relay_that_disappears_denies_its_open_cards() {
    let world = World::new();
    install_waiting(&world);
    let launch = world.launch("go");
    let end = drive(&world, launch, |_| async {
        let mut relay = Relay::connect(&world).await;
        relay.request(1, "Bash", json!({"command": "one"})).await;
        let card = wait_for_rows(&world, 1).await.remove(0);
        drop(relay);
        let row = wait_for_status(&world, &card.id, "denied").await;
        assert_eq!(row.answered_by.as_deref(), Some("stop"));
        finish_cli(&world);
    })
    .await;
    assert_eq!(end, RunEnd::Finished);
}

#[tokio::test]
async fn several_cards_are_answered_independently_and_only_pending_ones_can_be_answered() {
    let world = World::new();
    install_waiting(&world);
    let launch = world.launch("go");
    let end = drive(&world, launch, |_| async {
        let mut relay = Relay::connect(&world).await;
        relay.request(1, "Bash", json!({"command": "first"})).await;
        relay.request(2, "Bash", json!({"command": "second"})).await;
        let cards = wait_for_rows(&world, 2).await;
        let (first, second) = (&cards[0], &cards[1]);
        assert_eq!(first.summary, "first");
        answer(&world, &second.id, true).unwrap();
        let reply = relay.reply().await;
        assert_eq!(reply["id"], 2);
        assert_eq!(reply["behavior"], "allow");
        assert_eq!(rows(&world)[0].status, "pending");
        assert_eq!(
            answer(&world, &second.id, false).unwrap_err().code,
            "already-answered"
        );
        assert_eq!(rows(&world)[1].status, "allowed");
        assert_eq!(
            answer(&world, "no-such-card", true).unwrap_err().code,
            "not-found"
        );
        answer(&world, &first.id, false).unwrap();
        let reply = relay.reply().await;
        assert_eq!(reply["id"], 1);
        assert_eq!(reply["behavior"], "deny");
        finish_cli(&world);
    })
    .await;
    assert_eq!(end, RunEnd::Finished);
}

#[tokio::test]
async fn a_pending_row_whose_turn_is_not_running_cannot_be_answered() {
    let world = World::new();
    install_waiting(&world);
    let launch = world.launch("go");
    let turn_id = launch.turn_id.clone();
    world.db.with_connection(|c| {
        approval_store::insert(
            c,
            &approval_store::NewApproval {
                id: "orphan",
                turn_id: &turn_id,
                tool: "Bash",
                summary: "ls",
                detail: "{}",
                reason: None,
                created_at: 1,
            },
        )
        .unwrap();
    });
    assert_eq!(
        answer(&world, "orphan", true).unwrap_err().code,
        "not-running"
    );
    let row = world
        .db
        .with_connection(|c| approval_store::view(c, "orphan"))
        .unwrap()
        .unwrap();
    assert_eq!(row.status, "pending");
}

#[tokio::test]
async fn the_stall_timer_waits_while_a_card_is_open_and_starts_again_after_the_answer() {
    let world = World::new();
    install_waiting(&world);
    let mut launch = world.launch("go");
    launch.stall_timeout = Duration::from_millis(800);
    launch.tool_stall_timeout = Duration::from_millis(800);
    let answered_at = std::cell::Cell::new(None);
    let end = drive(&world, launch, |_| async {
        let mut relay = Relay::connect(&world).await;
        relay.request(1, "Bash", json!({"command": "ls"})).await;
        let card = wait_for_rows(&world, 1).await.remove(0);
        tokio::time::sleep(Duration::from_millis(2000)).await;
        assert_eq!(
            wait_for_status(&world, &card.id, "pending").await.status,
            "pending"
        );
        assert_eq!(only_turn(&world).status, "running");
        answer(&world, &card.id, true).unwrap();
        answered_at.set(Some(Instant::now()));
        let reply = relay.reply().await;
        assert_eq!(reply["behavior"], "allow");
    })
    .await;
    assert_eq!(end, RunEnd::Finished);
    assert!(
        answered_at.get().unwrap().elapsed() >= Duration::from_millis(600),
        "the stall timer must start again at the answer"
    );
    let turn = only_turn(&world);
    assert_eq!(turn.status, "failed");
    assert_eq!(
        turn.error.as_deref(),
        Some("No response from the provider for 1 minute.")
    );
}

#[tokio::test]
async fn the_summary_detail_and_events_of_a_card_hide_key_shaped_text() {
    let world = World::new();
    install_waiting(&world);
    let launch = world.launch("go");
    let command = format!("curl -H 'x-api-key: {LEAKY_TOKEN}' https://example.com");
    let end = drive(&world, launch, |_| async {
        let mut relay = Relay::connect(&world).await;
        relay
            .request(
                1,
                "Bash",
                json!({"command": command, "description": format!("uses {LEAKY_TOKEN}"), "extra": {"k": LEAKY_TOKEN}}),
            )
            .await;
        let card = wait_for_rows(&world, 1).await.remove(0);
        assert!(card.summary.contains("[redacted]"));
        assert!(!card.summary.contains("sk-ant"));
        assert!(!card.detail.contains("sk-ant"));
        assert!(!card.reason.clone().unwrap().contains("sk-ant"));
        answer(&world, &card.id, false).unwrap();
        relay.reply().await;
        finish_cli(&world);
    })
    .await;
    assert_eq!(end, RunEnd::Finished);
    assert!(!everything_emitted(&world).contains("sk-ant"));
}

#[tokio::test]
async fn a_long_input_is_bounded_in_the_summary_and_the_detail() {
    let world = World::new();
    install_waiting(&world);
    let launch = world.launch("go");
    let end = drive(&world, launch, |_| async {
        let mut relay = Relay::connect(&world).await;
        relay
            .request(1, "Bash", json!({"command": "x".repeat(20_000)}))
            .await;
        let card = wait_for_rows(&world, 1).await.remove(0);
        assert!(card.summary.len() < 700);
        assert!(card.detail.len() < 4200);
        answer(&world, &card.id, false).unwrap();
        relay.reply().await;
        finish_cli(&world);
    })
    .await;
    assert_eq!(end, RunEnd::Finished);
}

#[tokio::test]
async fn requests_past_the_open_limit_are_denied_without_a_card() {
    let world = World::new();
    install_waiting(&world);
    let launch = world.launch("go");
    let end = drive(&world, launch, |_| async {
        let mut relay = Relay::connect(&world).await;
        for id in 0..=approvals::MAX_OPEN as u64 {
            relay.request(id, "Bash", json!({"command": "ls"})).await;
        }
        let reply = relay.reply().await;
        assert_eq!(reply["behavior"], "deny");
        assert_eq!(reply["id"], approvals::MAX_OPEN as u64);
        assert_eq!(rows(&world).len(), approvals::MAX_OPEN);
        finish_cli(&world);
    })
    .await;
    assert_eq!(end, RunEnd::Finished);
}

#[test]
fn a_tool_without_a_target_is_described_by_its_name_and_other_tools_by_name_and_target() {
    let redactor = Redactor::none();
    let plain = approval::describe(&redactor, "Skill", &json!({"skill": "x"}));
    assert_eq!(plain.summary(), "Skill");
    let fetch = approval::describe(&redactor, "WebFetch", &json!({"url": "https://a.test/x"}));
    assert_eq!(fetch.summary(), "WebFetch https://a.test/x");
    let read = approval::describe(&redactor, "Read", &json!({"file_path": "/etc/hosts"}));
    assert_eq!(read.summary(), "Read /etc/hosts");
    let command = approval::describe(&redactor, "Bash", &json!({"command": "  ls -la  "}));
    assert_eq!(command.summary(), "ls -la");
    assert_eq!(command.reason(), None);
}

#[tokio::test]
async fn a_stop_denies_open_cards_at_once_even_when_the_cli_ignores_the_term_signal() {
    let world = World::new();
    install_stubborn(&world);
    let launch = world.launch("go");
    let world_ref = &world;
    let end = drive(&world, launch, |turn_id| async move {
        let world = world_ref;
        let mut relay = Relay::connect(world).await;
        relay.request(1, "Bash", json!({"command": "one"})).await;
        let card = wait_for_rows(world, 1).await.remove(0);
        let asked = Instant::now();
        let target = world.registry.request_stop("a", &turn_id).unwrap();
        tokio::spawn(kill_group_after(
            target.group.unwrap(),
            Duration::from_secs(2),
        ));
        let row = wait_for_status(world, &card.id, "denied").await;
        assert!(
            asked.elapsed() < Duration::from_millis(1200),
            "{:?}",
            asked.elapsed()
        );
        assert_eq!(row.answered_by.as_deref(), Some("stop"));
        let reply = relay.reply().await;
        assert_eq!(reply["behavior"], "deny");
    })
    .await;
    assert_eq!(end, RunEnd::Finished);
}

#[tokio::test]
async fn a_shutdown_signal_denies_open_cards_at_once_as_shutdown_even_when_the_cli_ignores_the_term_signal(
) {
    let world = World::new();
    install_stubborn(&world);
    let launch = world.launch("go");
    let world_ref = &world;
    let end = drive(&world, launch, |turn_id| async move {
        let world = world_ref;
        let mut relay = Relay::connect(world).await;
        relay.request(1, "Bash", json!({"command": "one"})).await;
        let card = wait_for_rows(world, 1).await.remove(0);
        let asked = Instant::now();
        world.registry.closing.store(true, Ordering::SeqCst);
        assert!(world
            .registry
            .send_signal(&turn_id, approvals::Message::Shutdown));
        let row = wait_for_status(world, &card.id, "denied").await;
        assert!(
            asked.elapsed() < Duration::from_millis(1200),
            "{:?}",
            asked.elapsed()
        );
        assert_eq!(row.answered_by.as_deref(), Some("shutdown"));
        assert_eq!(relay.reply().await["behavior"], "deny");
        world.registry.kill("a", &turn_id);
    })
    .await;
    assert_eq!(end, RunEnd::Closing);
}

#[tokio::test]
async fn a_cli_that_finishes_with_a_card_still_open_leaves_it_denied_not_pending() {
    let world = World::new();
    install_waiting(&world);
    let launch = world.launch("go");
    let end = drive(&world, launch, |_| async {
        let mut relay = Relay::connect(&world).await;
        relay.request(1, "Bash", json!({"command": "one"})).await;
        let card = wait_for_rows(&world, 1).await.remove(0);
        finish_cli(&world);
        let row = wait_for_status(&world, &card.id, "denied").await;
        assert_eq!(row.answered_by.as_deref(), Some("stop"));
        assert_eq!(relay.reply().await["behavior"], "deny");
    })
    .await;
    assert_eq!(end, RunEnd::Finished);
    assert!(rows(&world).iter().all(|row| row.status != "pending"));
}

#[tokio::test]
async fn a_request_that_arrives_after_a_stop_is_denied_at_once_without_a_card() {
    let world = World::new();
    install_stubborn(&world);
    let launch = world.launch("go");
    let world_ref = &world;
    let end = drive(&world, launch, |turn_id| async move {
        let world = world_ref;
        let mut relay = Relay::connect(world).await;
        wait_until_stubborn(world).await;
        let target = world.registry.request_stop("a", &turn_id).unwrap();
        tokio::spawn(kill_group_after(
            target.group.unwrap(),
            Duration::from_secs(2),
        ));
        tokio::time::sleep(Duration::from_millis(200)).await;
        relay.request(1, "Bash", json!({"command": "late"})).await;
        let reply = relay.reply().await;
        assert_eq!(reply["behavior"], "deny");
        assert_eq!(
            reply["message"],
            "The turn stopped, so this request was denied."
        );
        assert!(rows(world).is_empty());
    })
    .await;
    assert_eq!(end, RunEnd::Finished);
}

#[tokio::test]
async fn a_card_that_times_out_after_the_stall_limit_has_passed_does_not_stall_the_turn() {
    let world = World::new();
    install_waiting(&world);
    let mut launch = world.launch("go");
    launch.stall_timeout = Duration::from_millis(400);
    launch.tool_stall_timeout = Duration::from_millis(400);
    launch.approval_timeout = Duration::from_millis(1200);
    let end = drive(&world, launch, |_| async {
        let mut relay = Relay::connect(&world).await;
        relay.request(1, "Bash", json!({"command": "ls"})).await;
        let card = wait_for_rows(&world, 1).await.remove(0);
        assert_eq!(relay.reply().await["behavior"], "deny");
        wait_for_status(&world, &card.id, "timed-out").await;
        assert_eq!(only_turn(&world).status, "running");
        finish_cli(&world);
    })
    .await;
    assert_eq!(end, RunEnd::Finished);
    assert_eq!(only_turn(&world).status, "completed");
}
