use super::*;
use crate::turns::group::hand_offs::{LIMIT_NOTICE, MAX_TURNS};

impl Fixture {
    fn add_cy(&self) {
        let workspace = self.workspaces.path().join("Cy");
        fs::create_dir_all(&workspace).unwrap();
        self.db.with_connection(|c| {
            c.execute(
                "INSERT INTO agents (id, name, color, provider, model, workspace_path, created_at, last_activity_at)
                 VALUES ('c', 'Cy', 'red', 'claude-code', 'haiku', ?1, 1, 1)",
                [workspace.to_string_lossy()],
            )
            .unwrap();
            c.execute("INSERT INTO group_members VALUES ('g', 'c', 2, 1)", [])
                .unwrap();
        });
    }

    fn finish_with(&self, launch: &Launch, status: &str, reply: &str) {
        let group = launch.group.as_ref().unwrap().group_id.clone();
        self.say(
            &launch.agent_id,
            &group,
            &launch.turn_id,
            "assistant",
            Some(&launch.agent_id),
            reply,
        );
        self.end(launch, status);
    }

    fn next(&self, launch: &Launch) -> Option<Launch> {
        self.after_one(&launch.agent_id, Some("g"))
    }

    fn entries(&self) -> Vec<(String, String, String)> {
        self.db.with_connection(|c| {
            let mut statement = c
                .prepare(
                    "SELECT addressed_agent_id, kind, content FROM group_queue
                     WHERE group_id = 'g' ORDER BY position",
                )
                .unwrap();
            statement
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
                .unwrap()
                .map(Result::unwrap)
                .collect()
        })
    }

    fn notices(&self) -> Vec<String> {
        self.db.with_connection(|c| {
            let mut statement = c
                .prepare("SELECT content FROM messages WHERE role = 'notice' ORDER BY seq")
                .unwrap();
            statement
                .query_map([], |row| row.get(0))
                .unwrap()
                .map(Result::unwrap)
                .collect()
        })
    }

    fn turn_count(&self) -> i64 {
        self.value("SELECT COUNT(*) FROM turns WHERE group_id = 'g'", &[])
    }
}

fn entry(agent: &str, kind: &str, text: &str) -> (String, String, String) {
    (agent.to_owned(), kind.to_owned(), text.to_owned())
}

#[test]
fn a_user_mention_runs_the_mentioned_member_and_a_plain_message_still_goes_to_the_lead() {
    let fx = Fixture::new();
    let launch = fx.started("g", "@bo list the files");
    assert_eq!(launch.agent_id, "b");
    assert_eq!(input(&launch), "@bo list the files");
    fx.end(&launch, "completed");
    let plain = fx.started("g", "no mention, and @Zed is not a member");
    assert_eq!(plain.agent_id, "a");
}

#[test]
fn several_user_mentions_run_one_after_another_in_the_order_mentioned() {
    let fx = Fixture::new();
    let first = fx.started("g", "@Bo then @Ada, and @Bo again");
    assert_eq!(first.agent_id, "b");
    assert_eq!(fx.entries().len(), 1);
    assert_eq!(fx.entries()[0].0, "a");
    fx.end(&first, "completed");
    let second = fx.next(&first).unwrap();
    assert_eq!(second.agent_id, "a");
    assert!(fx.entries().is_empty());
    let roots: Vec<String> = fx.db.with_connection(|c| {
        let mut statement = c
            .prepare("SELECT root_message_id FROM turns ORDER BY rowid")
            .unwrap();
        statement
            .query_map([], |row| row.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    });
    assert_eq!(roots.len(), 2);
    assert_eq!(roots[0], roots[1]);
}

#[test]
fn mentions_in_a_user_message_queue_every_member_behind_a_busy_one() {
    let fx = Fixture::new();
    fx.started("g", "first");
    let launch = fx.queued("g", "@Bo and @Ada");
    assert!(launch.is_none());
    assert_eq!(
        fx.entries(),
        [
            entry("b", "user", "@Bo and @Ada"),
            entry("a", "user", "@Bo and @Ada")
        ]
    );
}

#[test]
fn a_hand_off_line_runs_the_member_next_and_the_agent_that_handed_off_then_continues() {
    let fx = Fixture::new();
    let first = fx.started("g", "hello");
    fx.finish_with(&first, "completed", "On it.\n@Bo check the README");
    assert_eq!(fx.entries(), [entry("b", "hand-off", "check the README")]);
    let second = fx.next(&first).unwrap();
    assert_eq!(second.agent_id, "b");
    assert!(second.text.contains("Ada: On it.\n@Bo check the README"));
    assert!(input(&second).ends_with("\n\ncheck the README"));
    assert_eq!(
        fx.value::<String>(
            "SELECT author_agent_id FROM messages WHERE turn_id = ?1 AND role = 'user'",
            &[&second.turn_id]
        ),
        "a"
    );
    assert_eq!(
        fx.value::<String>("SELECT kind FROM turns WHERE id = ?1", &[&second.turn_id]),
        "hand-off"
    );
    assert_eq!(
        fx.value::<i64>("SELECT hop FROM turns WHERE id = ?1", &[&second.turn_id]),
        1
    );
    fx.finish_with(&second, "completed", "The README is fine.");
    assert_eq!(
        fx.entries(),
        [entry("a", "continuation", "Continue with Bo's answer.")]
    );
    let third = fx.next(&second).unwrap();
    assert_eq!(third.agent_id, "a");
    assert!(third.text.contains("Bo: The README is fine."));
    assert!(input(&third).ends_with("\n\nContinue with Bo's answer."));
    assert_eq!(
        fx.value::<i64>("SELECT hop FROM turns WHERE id = ?1", &[&third.turn_id]),
        2
    );
    fx.finish_with(&third, "completed", "All done.");
    assert!(fx.entries().is_empty());
    assert!(fx.next(&third).is_none());
    assert_eq!(fx.turn_count(), 3);
    assert_eq!(
        fx.value::<i64>(
            "SELECT COUNT(DISTINCT root_message_id) FROM turns WHERE group_id = 'g'",
            &[]
        ),
        1
    );
}

#[test]
fn the_continuation_prompt_never_reaches_other_agents_and_a_hand_off_task_shows_its_author() {
    let fx = Fixture::new();
    let first = fx.started("g", "hello");
    fx.finish_with(&first, "completed", "@Bo check");
    let second = fx.next(&first).unwrap();
    fx.finish_with(&second, "completed", "checked");
    let third = fx.next(&second).unwrap();
    fx.finish_with(&third, "completed", "done");
    let again = fx.started("g", "@Bo again");
    assert_eq!(again.agent_id, "b");
    assert!(again.text.contains("Ada: done"));
    assert!(!again.text.contains("Continue with"));
    fx.end(&again, "completed");
    fx.add_cy();
    let cy = fx.started("g", "@Cy catch up");
    assert!(cy.text.contains("User: hello"));
    assert!(cy.text.contains("Ada: @Bo check"));
    assert!(cy.text.contains("Ada: check\n"));
    assert!(!cy.text.contains("Continue with"));
}

#[test]
fn several_hand_off_lines_queue_in_order_and_the_agent_continues_once_after_both_answer() {
    let fx = Fixture::new();
    fx.add_cy();
    let first = fx.started("g", "hello");
    fx.finish_with(&first, "completed", "Split:\n@Bo part one\n@Cy part two");
    assert_eq!(
        fx.entries(),
        [
            entry("b", "hand-off", "part one"),
            entry("c", "hand-off", "part two")
        ]
    );
    let bo = fx.next(&first).unwrap();
    fx.finish_with(&bo, "completed", "one done");
    assert_eq!(
        fx.entries(),
        [
            entry("c", "hand-off", "part two"),
            entry("a", "continuation", "Continue with Bo's answer.")
        ]
    );
    let cy = fx.next(&bo).unwrap();
    assert_eq!(cy.agent_id, "c");
    fx.finish_with(&cy, "completed", "two done");
    assert_eq!(
        fx.entries(),
        [entry("a", "continuation", "Continue with Bo's answer.")]
    );
    let ada = fx.next(&cy).unwrap();
    assert!(ada.text.contains("Bo: one done"));
    assert!(ada.text.contains("Cy: two done"));
}

#[test]
fn hand_offs_go_ahead_of_later_user_entries_of_the_same_message_and_of_other_messages() {
    let fx = Fixture::new();
    fx.add_cy();
    let first = fx.started("g", "@Ada start, then @Bo finish");
    fx.queued("g", "later message");
    assert_eq!(
        fx.entries(),
        [
            entry("b", "user", "@Ada start, then @Bo finish"),
            entry("a", "user", "later message")
        ]
    );
    fx.finish_with(&first, "completed", "@Cy dig in");
    assert_eq!(
        fx.entries(),
        [
            entry("c", "hand-off", "dig in"),
            entry("b", "user", "@Ada start, then @Bo finish"),
            entry("a", "user", "later message")
        ]
    );
    let positions: Vec<i64> = fx.db.with_connection(|c| {
        let mut statement = c
            .prepare("SELECT position FROM group_queue ORDER BY position")
            .unwrap();
        statement
            .query_map([], |row| row.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    });
    assert_eq!(positions, [0, 1, 2]);
}

#[test]
fn a_reply_that_mentions_only_itself_a_non_member_or_a_code_block_hands_nothing_off() {
    let fx = Fixture::new();
    let first = fx.started("g", "hello");
    fx.finish_with(
        &first,
        "completed",
        "@Ada me again\n@Zed nobody\nrun:\n```\n@Bo no\n```\nI told @Bo about it",
    );
    assert!(fx.entries().is_empty());
    assert!(fx.next(&first).is_none());
}

#[test]
fn only_the_final_reply_text_of_a_turn_is_parsed() {
    let fx = Fixture::new();
    let first = fx.started("g", "hello");
    fx.say(
        "a",
        "g",
        &first.turn_id,
        "assistant",
        Some("a"),
        "@Bo early text",
    );
    fx.finish_with(&first, "completed", "final text without a hand-off");
    assert!(fx.entries().is_empty());
}

#[test]
fn a_stopped_failed_or_interrupted_turn_hands_nothing_off_and_queues_no_continuation() {
    for status in ["stopped", "failed"] {
        let fx = Fixture::new();
        let first = fx.started("g", "hello");
        fx.finish_with(&first, status, "@Bo check the README");
        assert!(fx.entries().is_empty(), "{status}");
        assert!(fx.next(&first).is_none(), "{status}");

        let fx = Fixture::new();
        let first = fx.started("g", "hello");
        fx.finish_with(&first, "completed", "@Bo check the README");
        let second = fx.next(&first).unwrap();
        fx.finish_with(&second, status, "partial answer");
        assert!(fx.entries().is_empty(), "{status}");
        assert!(fx.next(&second).is_none(), "{status}");
    }
    let fx = Fixture::new();
    let first = fx.started("g", "hello");
    fx.say(
        "a",
        "g",
        &first.turn_id,
        "assistant",
        Some("a"),
        "@Bo check the README",
    );
    fx.db
        .with_connection(|c| super::super::super::store::sweep_interrupted(c, 60))
        .unwrap();
    assert!(fx.entries().is_empty());
}

#[test]
fn stopping_keeps_the_entries_already_queued() {
    let fx = Fixture::new();
    let first = fx.started("g", "hello");
    fx.queued("g", "waiting");
    fx.finish_with(&first, "stopped", "@Bo dropped");
    assert_eq!(fx.entries(), [entry("a", "user", "waiting")]);
}

#[test]
fn a_chain_stops_at_eight_agent_turns_with_the_limit_notice() {
    let fx = Fixture::new();
    fx.add_cy();
    let ring = |agent: &str| match agent {
        "a" => "Bo",
        "b" => "Cy",
        _ => "Ada",
    };
    let mut launch = fx.started("g", "hello");
    loop {
        let reply = format!("@{} keep going", ring(&launch.agent_id));
        fx.finish_with(&launch, "completed", &reply);
        match fx.next(&launch) {
            Some(next) => launch = next,
            None => break,
        }
        assert!(fx.turn_count() <= MAX_TURNS);
    }
    assert_eq!(fx.turn_count(), MAX_TURNS);
    assert!(fx.entries().is_empty());
    assert_eq!(fx.notices(), [LIMIT_NOTICE]);
}

#[test]
fn at_the_limit_no_hand_off_is_queued_the_notice_comes_once_and_user_entries_stay() {
    let fx = Fixture::new();
    fx.add_cy();
    let first = fx.started("g", "@Ada go");
    fx.queued("g", "later user message");
    fx.db.with_connection(|c| {
        for n in 0..7 {
            c.execute(
                "INSERT INTO turns (id, agent_id, group_id, status, started_at, root_message_id)
                 SELECT ?1, 'b', 'g', 'completed', 5, root_message_id FROM turns WHERE id = ?2",
                params![format!("old{n}"), first.turn_id],
            )
            .unwrap();
        }
    });
    fx.finish_with(&first, "completed", "@Bo one\n@Cy two");
    assert_eq!(fx.notices(), [LIMIT_NOTICE]);
    assert_eq!(fx.entries(), [entry("a", "user", "later user message")]);
}

#[test]
fn a_hand_off_that_still_fits_under_the_limit_counts_the_entries_already_waiting() {
    let fx = Fixture::new();
    fx.add_cy();
    let first = fx.started("g", "@Ada go");
    fx.db.with_connection(|c| {
        for n in 0..5 {
            c.execute(
                "INSERT INTO turns (id, agent_id, group_id, status, started_at, root_message_id)
                 SELECT ?1, 'b', 'g', 'completed', 5, root_message_id FROM turns WHERE id = ?2",
                params![format!("old{n}"), first.turn_id],
            )
            .unwrap();
        }
    });
    fx.finish_with(&first, "completed", "@Bo one\n@Cy two\n@Bo three");
    assert_eq!(fx.entries().len(), 2);
    assert_eq!(fx.notices(), [LIMIT_NOTICE]);
}

#[test]
fn a_third_hand_off_between_the_same_two_agents_is_not_queued_and_posts_a_notice() {
    let fx = Fixture::new();
    let first = fx.started("g", "hello");
    fx.finish_with(&first, "completed", "@Bo please");
    let second = fx.next(&first).unwrap();
    assert_eq!(second.agent_id, "b");
    fx.finish_with(&second, "completed", "@Ada back to you");
    assert_eq!(fx.entries(), [entry("a", "hand-off", "back to you")]);
    assert!(fx.notices().is_empty());
    let third = fx.next(&second).unwrap();
    assert_eq!(third.agent_id, "a");
    fx.finish_with(&third, "completed", "@Bo once more");
    assert_eq!(
        fx.notices(),
        ["Ada and Bo keep handing work back; that hand-off was not started."]
    );
    assert_eq!(
        fx.entries(),
        [entry("b", "continuation", "Continue with Ada's answer.")]
    );
}

#[test]
fn two_hand_offs_in_a_row_between_the_same_agents_are_allowed() {
    let fx = Fixture::new();
    let first = fx.started("g", "hello");
    fx.finish_with(&first, "completed", "@Bo please");
    let second = fx.next(&first).unwrap();
    fx.finish_with(&second, "completed", "@Ada back to you");
    assert_eq!(fx.entries().len(), 1);
    assert!(fx.notices().is_empty());
}

#[test]
fn the_hand_off_path_takes_the_lower_access_level_of_the_two_agents() {
    assert_eq!(
        super::super::access_for_hand_off("a", "b"),
        AccessLevel::STANDARD
    );
    assert_eq!(
        super::super::lower_access(AccessLevel(0), AccessLevel::STANDARD),
        AccessLevel(0)
    );
    assert_eq!(
        super::super::lower_access(AccessLevel::STANDARD, AccessLevel(0)),
        AccessLevel(0)
    );
    let fx = Fixture::new();
    let first = fx.started("g", "hello");
    fx.finish_with(&first, "completed", "@Bo check");
    let second = fx.next(&first).unwrap();
    assert_eq!(second.group.as_ref().unwrap().access, AccessLevel::STANDARD);
}

#[tokio::test]
async fn the_runner_queues_a_hand_off_and_announces_the_limit_notice_when_a_turn_completes() {
    let fx = Fixture::new();
    let reply = "Done here.\\n@Bo check the README";
    let scripted = BASIC
        .replace("\"text\":\"po\"", &format!("\"text\":\"{reply}\""))
        .replace("\"text\":\"ng\"", "\"text\":\"\"")
        .replace("\"text\":\"pong\"", &format!("\"text\":\"{reply}\""))
        .replace("\"result\":\"pong\"", &format!("\"result\":\"{reply}\""));
    let file = fx.records.path().join("scripted.jsonl");
    fs::write(&file, scripted).unwrap();
    fx.install(&format!("cat \"{}\"; exit 0", file.display()));
    let first = fx.started("g", "hello");
    fx.run(first).await;
    assert_eq!(fx.entries(), [entry("b", "hand-off", "check the README")]);
    assert!(fx.notices().is_empty());

    let fx2 = Fixture::new();
    fx2.install(&format!("cat \"{}\"; exit 0", file.display()));
    let first = fx2.started("g", "hello");
    fx2.db.with_connection(|c| {
        for n in 0..7 {
            c.execute(
                "INSERT INTO turns (id, agent_id, group_id, status, started_at, root_message_id)
                 SELECT ?1, 'b', 'g', 'completed', 5, root_message_id FROM turns WHERE id = ?2",
                params![format!("old{n}"), first.turn_id],
            )
            .unwrap();
        }
    });
    fx2.run(first).await;
    assert!(fx2.entries().is_empty());
    assert_eq!(fx2.notices(), [LIMIT_NOTICE]);
    let events = fx2.sink.0.lock().unwrap();
    assert!(events.iter().any(|envelope| matches!(
        &envelope.event,
        crate::turns::TurnEvent::Notice { text, .. } if text == LIMIT_NOTICE
    )));
}

#[test]
fn the_conversation_tells_a_hand_off_or_continuation_turn_from_a_user_turn() {
    let fx = Fixture::new();
    let first = fx.started("g", "hello");
    fx.finish_with(&first, "completed", "@Bo check");
    let second = fx.next(&first).unwrap();
    fx.finish_with(&second, "completed", "checked");
    let third = fx.next(&second).unwrap();
    fx.end(&third, "completed");
    let page = fx
        .db
        .with_connection(|c| store::group_conversation(c, "g", None))
        .unwrap();
    let seen: Vec<(String, Option<String>)> = page
        .turns
        .into_iter()
        .map(|turn| (turn.kind, turn.from_agent_id))
        .collect();
    assert_eq!(
        seen,
        [
            ("user".to_owned(), None),
            ("hand-off".to_owned(), Some("a".to_owned())),
            ("continuation".to_owned(), Some("b".to_owned())),
        ]
    );
}
