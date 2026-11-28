use super::*;
use crate::turns::group::hand_offs::{LIMIT_NOTICE, MAX_TURNS, STOPPED_NOTICE};

impl Fixture {
    fn add_agent(&self, id: &str, name: &str, position: i64) {
        let workspace = self.workspaces.path().join(name);
        fs::create_dir_all(&workspace).unwrap();
        self.db.with_connection(|c| {
            c.execute(
                "INSERT INTO agents (id, name, color, provider, model, workspace_path, created_at, last_activity_at)
                 VALUES (?1, ?2, 'red', 'claude-code', 'haiku', ?3, 1, 1)",
                params![id, name, workspace.to_string_lossy()],
            )
            .unwrap();
            c.execute(
                "INSERT INTO group_members VALUES ('g', ?1, ?2, 1)",
                params![id, position],
            )
            .unwrap();
        });
    }

    fn add_cy(&self) {
        self.add_agent("c", "Cy", 2);
    }

    fn add_dee(&self) {
        self.add_agent("d", "Dee", 3);
    }

    fn marks(&self, turn_id: &str) -> Vec<(String, String)> {
        let page = self
            .db
            .with_connection(|c| store::group_conversation(c, "g", None))
            .unwrap();
        page.turns
            .into_iter()
            .find(|turn| turn.id == turn_id)
            .unwrap()
            .hand_offs
            .into_iter()
            .map(|mark| (mark.agent_id, mark.task))
            .collect()
    }

    fn order(&self, launch: Launch, replies: &[(&str, &str)]) -> Vec<String> {
        let mut seen = vec![launch.agent_id.clone()];
        let mut current = launch;
        for (status, reply) in replies {
            self.finish_with(&current, status, reply);
            match self.next(&current) {
                Some(next) => {
                    seen.push(next.agent_id.clone());
                    current = next;
                }
                None => break,
            }
        }
        seen
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
    assert_eq!(
        fx.marks(&first.turn_id),
        [
            ("b".to_owned(), "part one".to_owned()),
            ("c".to_owned(), "part two".to_owned())
        ]
    );
    let bo = fx.next(&first).unwrap();
    fx.finish_with(&bo, "completed", "one done");
    assert_eq!(fx.entries(), [entry("c", "hand-off", "part two")]);
    let cy = fx.next(&bo).unwrap();
    assert_eq!(cy.agent_id, "c");
    fx.finish_with(&cy, "completed", "two done");
    assert_eq!(
        fx.entries(),
        [entry("a", "continuation", "Continue with Cy's answer.")]
    );
    let ada = fx.next(&cy).unwrap();
    assert!(ada.text.contains("Bo: one done"));
    assert!(ada.text.contains("Cy: two done"));
}

#[test]
fn a_nested_hand_off_returns_depth_first_each_agent_continuing_for_the_one_above() {
    let fx = Fixture::new();
    fx.add_cy();
    let first = fx.started("g", "hello");
    let order = fx.order(
        first,
        &[
            ("completed", "@Bo ask Cy"),
            ("completed", "@Cy dig"),
            ("completed", "dug"),
            ("completed", "Bo wraps up"),
            ("completed", "Ada wraps up"),
        ],
    );
    assert_eq!(order, ["a", "b", "c", "b", "a"]);
    assert!(fx.entries().is_empty());
    assert_eq!(fx.turn_count(), 5);
}

#[test]
fn a_continuation_for_the_top_agent_carries_the_final_answer_of_the_agent_below() {
    let fx = Fixture::new();
    fx.add_cy();
    let first = fx.started("g", "hello");
    fx.finish_with(&first, "completed", "@Bo ask Cy");
    let bo = fx.next(&first).unwrap();
    fx.finish_with(&bo, "completed", "@Cy dig");
    let cy = fx.next(&bo).unwrap();
    fx.finish_with(&cy, "completed", "dug");
    assert_eq!(
        fx.entries(),
        [entry("b", "continuation", "Continue with Cy's answer.")]
    );
    let bo_again = fx.next(&cy).unwrap();
    fx.finish_with(&bo_again, "completed", "Bo's final answer");
    assert_eq!(
        fx.entries(),
        [entry("a", "continuation", "Continue with Bo's answer.")]
    );
    let ada = fx.next(&bo_again).unwrap();
    assert!(ada.text.contains("Bo: Bo's final answer"));
    assert!(input(&ada).ends_with("Continue with Bo's answer."));
}

#[test]
fn siblings_with_a_nested_hand_off_run_b_d_b_c_a() {
    let fx = Fixture::new();
    fx.add_cy();
    fx.add_dee();
    let first = fx.started("g", "hello");
    let order = fx.order(
        first,
        &[
            ("completed", "@Bo part one\n@Cy part two"),
            ("completed", "@Dee help"),
            ("completed", "helped"),
            ("completed", "one done"),
            ("completed", "two done"),
            ("completed", "all done"),
        ],
    );
    assert_eq!(order, ["a", "b", "d", "b", "c", "a"]);
    assert!(fx.entries().is_empty());
    assert_eq!(fx.turn_count(), 6);
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

        if status == "stopped" {
            let fx = Fixture::new();
            let first = fx.started("g", "hello");
            fx.finish_with(&first, "completed", "@Bo check the README");
            let second = fx.next(&first).unwrap();
            fx.finish_with(&second, status, "partial answer");
            assert!(fx.entries().is_empty(), "{status}");
            assert!(fx.next(&second).is_none(), "{status}");
        }
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

fn all_continue(fx: &Fixture, replies: &[&str], expected: &[&str]) {
    let first = fx.started("g", "hello");
    let steps: Vec<(&str, &str)> = replies.iter().map(|reply| ("completed", *reply)).collect();
    assert_eq!(fx.order(first, &steps), expected);
    assert!(fx.entries().is_empty());
    assert_eq!(fx.turn_count(), expected.len() as i64);
    assert!(fx.notices().is_empty());
}

#[test]
fn an_agent_with_two_open_debts_in_one_message_continues_for_each() {
    let fx = Fixture::new();
    fx.add_cy();
    all_continue(
        &fx,
        &[
            "@Bo ask",
            "@Ada back",
            "@Cy dig",
            "dug",
            "A2 done",
            "B done",
            "A1 done",
        ],
        &["a", "b", "a", "c", "a", "b", "a"],
    );
}

#[test]
fn siblings_with_a_back_hand_off_and_a_nested_one_each_continue_once_in_depth_first_order() {
    let fx = Fixture::new();
    fx.add_cy();
    fx.add_dee();
    all_continue(
        &fx,
        &[
            "@Bo p1\n@Cy p2",
            "@Ada q",
            "@Dee r",
            "dee done",
            "A2 done",
            "B done",
            "C done",
            "A1 done",
        ],
        &["a", "b", "a", "d", "a", "b", "c", "a"],
    );
}

#[test]
fn a_failed_grandchild_lets_its_parent_continue_with_a_notice_and_the_chain_go_on() {
    let fx = Fixture::new();
    fx.add_cy();
    let first = fx.started("g", "hello");
    fx.finish_with(&first, "completed", "@Bo ask Cy");
    let bo = fx.next(&first).unwrap();
    fx.finish_with(&bo, "completed", "@Cy dig");
    let cy = fx.next(&bo).unwrap();
    fx.finish_with(&cy, "failed", "");
    assert_eq!(
        fx.entries(),
        [entry("b", "continuation", "Continue without Cy's answer.")]
    );
    let bo_again = fx.next(&cy).unwrap();
    assert_eq!(bo_again.agent_id, "b");
    assert_eq!(
        fx.notices(),
        ["Cy did not finish; continuing without its answer."]
    );
    fx.finish_with(&bo_again, "completed", "Bo wraps up");
    let ada = fx.next(&bo_again).unwrap();
    assert_eq!(ada.agent_id, "a");
    assert!(ada.text.contains("Bo: Bo wraps up"));
}

#[test]
fn an_interrupted_grandchild_is_settled_by_the_startup_sweep_and_the_parent_waits_for_resume() {
    let fx = Fixture::new();
    fx.add_cy();
    let first = fx.started("g", "hello");
    fx.finish_with(&first, "completed", "@Bo ask Cy");
    let bo = fx.next(&first).unwrap();
    fx.finish_with(&bo, "completed", "@Cy dig");
    let cy = fx.next(&bo).unwrap();
    fx.registry.clear(&cy.agent_id, &cy.turn_id);
    fx.db
        .with_connection(|c| super::super::super::store::sweep_interrupted(c, 60))
        .unwrap();
    assert_eq!(
        fx.entries(),
        [entry("b", "continuation", "Continue without Cy's answer.")]
    );
    let resumed = resume_group(&fx.db, &fx.registry, &fx.dirs(), "g")
        .unwrap()
        .unwrap();
    assert_eq!(resumed.agent_id, "b");
    assert_eq!(
        fx.notices(),
        ["Cy did not finish; continuing without its answer."]
    );
}

#[test]
fn a_member_removed_mid_chain_resolves_its_parents_debt() {
    let fx = Fixture::new();
    let first = fx.started("g", "hello");
    fx.finish_with(&first, "completed", "@Bo check");
    assert_eq!(fx.entries(), [entry("b", "hand-off", "check")]);
    fx.db.with_connection(|c| {
        crate::groups::store::replace_members(c, "g", &["a".to_owned()], 60).unwrap();
        super::super::drop_for_departed(c, Some("g"), "b", 60).unwrap();
    });
    assert_eq!(
        fx.entries(),
        [entry("a", "continuation", "Continue without Bo's answer.")]
    );
}

#[test]
fn a_deleted_agent_resolves_its_waiting_hand_off_and_a_parent_that_left_passes_the_debt_up() {
    let fx = Fixture::new();
    fx.add_cy();
    let first = fx.started("g", "hello");
    fx.finish_with(&first, "completed", "@Bo ask Cy");
    let bo = fx.next(&first).unwrap();
    fx.finish_with(&bo, "completed", "@Cy dig");
    fx.db.with_connection(|c| {
        super::super::drop_for_departed(c, None, "b", 60).unwrap();
    });
    assert_eq!(fx.entries(), [entry("c", "hand-off", "dig")]);
    let cy = fx.next(&bo).unwrap();
    fx.finish_with(&cy, "completed", "dug");
    assert_eq!(
        fx.entries(),
        [entry("a", "continuation", "Continue without Bo's answer.")]
    );
}

#[test]
fn an_edited_queued_message_is_stored_and_shown_and_reaches_other_agents() {
    let fx = Fixture::new();
    fx.add_cy();
    let first = fx.started("g", "@Bo look then @Ada decide");
    let queued = fx.db.with_connection(|c| list_queue(c, "g")).unwrap();
    fx.db
        .with_connection(|c| edit_queued(c, &queued[0].id, "@Ada decide differently"))
        .unwrap();
    fx.finish_with(&first, "completed", "looked");
    let second = fx.next(&first).unwrap();
    assert_eq!(second.agent_id, "a");
    fx.finish_with(&second, "completed", "decided");
    let page = fx
        .db
        .with_connection(|c| store::group_conversation(c, "g", None))
        .unwrap();
    let bubbles: Vec<&str> = page
        .turns
        .iter()
        .map(|turn| turn.user_message.as_str())
        .collect();
    assert_eq!(
        bubbles,
        ["@Bo look then @Ada decide", "@Ada decide differently"]
    );
    let cy = fx.started("g", "@Cy catch up");
    assert!(cy.text.contains("User: @Ada decide differently"));
    assert_eq!(
        cy.text.matches("User: @Bo look then @Ada decide").count(),
        1
    );
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
    assert!(fx.marks(&first.turn_id).is_empty());
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
    assert!(fx.marks(&third.turn_id).is_empty());
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
        super::super::access_for_hand_off(AccessLevel::STANDARD, "b"),
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
}

#[test]
fn a_lower_access_level_carries_down_a_two_hop_chain() {
    let fx = Fixture::new();
    fx.add_cy();
    super::super::set_test_level("b", AccessLevel(0));
    let first = fx.started("g", "hello");
    assert_eq!(first.group.as_ref().unwrap().access, AccessLevel::STANDARD);
    fx.finish_with(&first, "completed", "@Bo ask Cy");
    let bo = fx.next(&first).unwrap();
    assert_eq!(bo.group.as_ref().unwrap().access, AccessLevel(0));
    fx.finish_with(&bo, "completed", "@Cy dig");
    let cy = fx.next(&bo).unwrap();
    assert_eq!(cy.agent_id, "c");
    assert_eq!(cy.group.as_ref().unwrap().access, AccessLevel(0));
    fx.finish_with(&cy, "completed", "dug");
    let bo_again = fx.next(&cy).unwrap();
    assert_eq!(bo_again.group.as_ref().unwrap().access, AccessLevel(0));
    fx.finish_with(&bo_again, "completed", "done");
    let ada = fx.next(&bo_again).unwrap();
    assert_eq!(ada.group.as_ref().unwrap().access, AccessLevel(0));
}

#[tokio::test]
async fn the_carried_access_level_reaches_the_cli_arguments_through_a_two_hop_chain() {
    let fx = Fixture::new();
    fx.add_cy();
    super::super::set_test_level("b", AccessLevel(0));
    let script = |reply: &str| {
        BASIC
            .replace("\"text\":\"po\"", &format!("\"text\":\"{reply}\""))
            .replace("\"text\":\"ng\"", "\"text\":\"\"")
            .replace("\"text\":\"pong\"", &format!("\"text\":\"{reply}\""))
            .replace("\"result\":\"pong\"", &format!("\"result\":\"{reply}\""))
    };
    fs::write(fx.records.path().join("s0.jsonl"), script("@Bo ask Cy")).unwrap();
    fs::write(fx.records.path().join("s1.jsonl"), script("@Cy dig")).unwrap();
    let records = fx.records.path().display();
    fx.install(&format!(
        "[ \"$n\" = 0 ] && cat \"{records}/s0.jsonl\" && exit 0\n[ \"$n\" = 1 ] && cat \"{records}/s1.jsonl\" && exit 0"
    ));
    let first = fx.started("g", "hello");
    fx.run(first).await;
    let bo = fx.after_one("a", Some("g")).unwrap();
    assert_eq!(bo.agent_id, "b");
    fx.run(bo).await;
    let cy = fx.after_one("b", Some("g")).unwrap();
    assert_eq!(cy.agent_id, "c");
    fx.run(cy).await;
    let mode = |n: usize| {
        let args = fx.args(n);
        let at = args
            .iter()
            .position(|arg| arg == "--permission-mode")
            .unwrap();
        args[at + 1].clone()
    };
    assert_eq!(mode(0), "manual");
    assert_eq!(mode(1), "manual-lowest");
    assert_eq!(mode(2), "manual-lowest");
}

#[test]
fn a_stopped_turn_drops_every_waiting_hand_off_of_its_message_and_says_so() {
    let fx = Fixture::new();
    fx.add_cy();
    let first = fx.started("g", "hello");
    fx.finish_with(&first, "completed", "@Bo part one\n@Cy part two");
    let bo = fx.next(&first).unwrap();
    fx.finish_with(&bo, "stopped", "partial");
    assert!(fx.entries().is_empty());
    assert_eq!(
        fx.marks(&first.turn_id),
        [("b".to_owned(), "part one".to_owned())]
    );
    assert_eq!(fx.notices(), [STOPPED_NOTICE]);
    assert!(fx.next(&bo).is_none());
    assert_eq!(fx.turn_count(), 2);
}

#[test]
fn stopping_a_nested_chain_drops_the_entries_above_it_and_keeps_user_entries() {
    let fx = Fixture::new();
    fx.add_cy();
    fx.add_dee();
    let first = fx.started("g", "@Ada start, then @Bo user entry");
    fx.queued("g", "another message");
    fx.finish_with(&first, "completed", "@Cy dig\n@Dee also");
    let cy = fx.next(&first).unwrap();
    assert_eq!(cy.agent_id, "c");
    fx.finish_with(&cy, "completed", "@Bo deeper");
    let bo = fx.next(&cy).unwrap();
    assert_eq!(bo.agent_id, "b");
    fx.finish_with(&bo, "stopped", "partial");
    assert_eq!(
        fx.entries(),
        [
            entry("b", "user", "@Ada start, then @Bo user entry"),
            entry("a", "user", "another message")
        ]
    );
    assert_eq!(fx.notices(), [STOPPED_NOTICE]);
    let user = fx.next(&bo).unwrap();
    assert_eq!(user.agent_id, "b");
}

#[test]
fn stopping_a_turn_with_nothing_waiting_posts_no_notice() {
    let fx = Fixture::new();
    let first = fx.started("g", "hello");
    fx.finish_with(&first, "stopped", "partial");
    assert!(fx.notices().is_empty());
}

#[test]
fn a_message_addresses_at_most_eight_members_and_the_first_turn_says_so() {
    let fx = Fixture::new();
    for n in 3..=9 {
        fx.add_agent(&format!("m{n}"), &format!("M{n}"), n);
    }
    let text = "@Ada @Bo @M3 @M4 @M5 @M6 @M7 @M8 @M9";
    let first = fx.started("g", text);
    assert_eq!(first.agent_id, "a");
    assert_eq!(fx.entries().len(), 7);
    assert!(fx.entries().iter().all(|(agent, _, _)| agent != "m9"));
    assert_eq!(fx.notices(), [TRUNCATED_NOTICE]);
}

#[test]
fn the_user_message_is_stored_once_and_reaches_each_agent_once() {
    let fx = Fixture::new();
    let first = fx.started("g", "@Bo look then @Ada decide");
    assert_eq!(first.agent_id, "b");
    fx.finish_with(&first, "completed", "looked");
    let second = fx.next(&first).unwrap();
    assert_eq!(second.agent_id, "a");
    assert_eq!(
        fx.value::<i64>(
            "SELECT COUNT(*) FROM messages WHERE group_id = 'g' AND role = 'user'",
            &[]
        ),
        1
    );
    assert_eq!(second.text.matches("look then @Ada decide").count(), 1);
    assert!(second.text.contains("Bo: looked"));
    fx.finish_with(&second, "completed", "decided");
    let page = fx
        .db
        .with_connection(|c| store::group_conversation(c, "g", None))
        .unwrap();
    let bubbles: Vec<&str> = page
        .turns
        .iter()
        .map(|turn| turn.user_message.as_str())
        .collect();
    assert_eq!(bubbles, ["@Bo look then @Ada decide", ""]);
    fx.add_cy();
    let later = fx.started("g", "@Cy once more");
    assert_eq!(later.text.matches("look then @Ada decide").count(), 1);
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

fn crafted(fx: &Fixture, parent_deferred: i64, sibling_deferred: i64) {
    fx.db.with_connection(|c| {
        c.execute(
            "INSERT INTO turns (id, agent_id, group_id, status, started_at, root_message_id, deferred)
             VALUES ('p', 'a', 'g', 'completed', 1, 'r', ?1)",
            [parent_deferred],
        )
        .unwrap();
        c.execute(
            "INSERT INTO turns (id, agent_id, group_id, status, started_at, root_message_id, deferred, parent_turn_id, kind)
             VALUES ('s', 'b', 'g', 'completed', 2, 'r', ?1, 'p', 'hand-off')",
            [sibling_deferred],
        )
        .unwrap();
        c.execute(
            "INSERT INTO turns (id, agent_id, group_id, status, started_at, root_message_id, parent_turn_id, kind)
             VALUES ('c', 'b', 'g', 'failed', 3, 'r', 'p', 'hand-off')",
            [],
        )
        .unwrap();
        super::super::hand_offs::unfinished_after(c, "c", 60).unwrap();
    });
}

#[test]
fn a_parent_is_not_resolved_while_a_sibling_is_itself_waiting_on_its_own_children() {
    let fx = Fixture::new();
    crafted(&fx, 1, 1);
    assert!(fx.entries().is_empty());
    let fx = Fixture::new();
    crafted(&fx, 1, 0);
    assert_eq!(fx.entries().len(), 1);
}

#[test]
fn a_parent_that_is_no_longer_owed_gets_no_second_continuation() {
    let fx = Fixture::new();
    crafted(&fx, 0, 0);
    assert!(fx.entries().is_empty());
}

impl Fixture {
    fn remove_queued_entry(&self, index: usize) {
        let queued = self.db.with_connection(|c| list_queue(c, "g")).unwrap();
        self.db
            .with_connection(|c| remove_queued(c, &queued[index].id, 60))
            .unwrap();
    }
}

#[test]
fn removing_a_waiting_hand_off_lets_the_parent_continue_once_with_a_notice() {
    let fx = Fixture::new();
    fx.add_cy();
    let first = fx.started("g", "hello");
    fx.finish_with(&first, "completed", "@Bo part one\n@Cy part two");
    let bo = fx.next(&first).unwrap();
    fx.finish_with(&bo, "completed", "one done");
    assert_eq!(fx.entries(), [entry("c", "hand-off", "part two")]);
    fx.remove_queued_entry(0);
    assert_eq!(
        fx.entries(),
        [entry("a", "continuation", "Continue without Cy's answer.")]
    );
    let ada = fx.next(&bo).unwrap();
    assert_eq!(ada.agent_id, "a");
    assert!(ada.text.contains("Bo: one done"));
    assert_eq!(
        fx.notices(),
        ["Cy's task was removed; continuing without its answer."]
    );
    fx.finish_with(&ada, "completed", "done");
    assert!(fx.next(&ada).is_none());
    assert_eq!(fx.turn_count(), 3);
}

#[test]
fn removing_a_waiting_continuation_in_a_nested_chain_lets_the_grandparent_continue() {
    let fx = Fixture::new();
    fx.add_cy();
    let first = fx.started("g", "hello");
    fx.finish_with(&first, "completed", "@Bo ask Cy");
    let bo = fx.next(&first).unwrap();
    fx.finish_with(&bo, "completed", "@Cy dig");
    let cy = fx.next(&bo).unwrap();
    fx.finish_with(&cy, "completed", "dug");
    assert_eq!(
        fx.entries(),
        [entry("b", "continuation", "Continue with Cy's answer.")]
    );
    fx.remove_queued_entry(0);
    assert_eq!(
        fx.entries(),
        [entry("a", "continuation", "Continue without Bo's answer.")]
    );
    let ada = fx.next(&cy).unwrap();
    assert_eq!(ada.agent_id, "a");
    assert_eq!(
        fx.notices(),
        ["Bo's task was removed; continuing without its answer."]
    );
}

#[test]
fn removing_a_user_entry_settles_nothing() {
    let fx = Fixture::new();
    let first = fx.started("g", "hello");
    fx.queued("g", "waiting");
    fx.remove_queued_entry(0);
    assert!(fx.entries().is_empty());
    fx.end(&first, "completed");
}

#[test]
fn a_departed_child_that_leaves_the_parent_open_hands_its_notice_to_the_eventual_continuation() {
    let fx = Fixture::new();
    fx.add_cy();
    fx.add_dee();
    let first = fx.started("g", "hello");
    fx.finish_with(&first, "completed", "@Bo one\n@Cy two\n@Dee three");
    fx.db.with_connection(|c| {
        crate::groups::store::replace_members(
            c,
            "g",
            &["a".to_owned(), "c".to_owned(), "d".to_owned()],
            60,
        )
        .unwrap();
        super::super::drop_for_departed(c, Some("g"), "b", 60).unwrap();
    });
    assert_eq!(fx.entries().len(), 2);
    let cy = fx.next(&first).unwrap();
    assert_eq!(cy.agent_id, "c");
    fx.finish_with(&cy, "failed", "");
    let dee = fx.next(&cy).unwrap();
    assert_eq!(dee.agent_id, "d");
    fx.finish_with(&dee, "completed", "three done");
    let ada = fx.next(&dee).unwrap();
    assert_eq!(ada.agent_id, "a");
    assert_eq!(
        fx.notices(),
        [
            "Bo did not finish; continuing without its answer.",
            "Cy did not finish; continuing without its answer."
        ]
    );
}

#[test]
fn a_departed_child_at_the_limit_posts_the_limit_notice() {
    let fx = Fixture::new();
    fx.add_cy();
    let first = fx.started("g", "hello");
    fx.finish_with(&first, "completed", "@Bo one");
    fx.db.with_connection(|c| {
        for n in 0..7 {
            c.execute(
                "INSERT INTO turns (id, agent_id, group_id, status, started_at, root_message_id)
                 SELECT ?1, 'c', 'g', 'completed', 5, root_message_id FROM turns WHERE id = ?2",
                params![format!("old{n}"), first.turn_id],
            )
            .unwrap();
        }
        crate::groups::store::replace_members(c, "g", &["a".to_owned(), "c".to_owned()], 60)
            .unwrap();
        super::super::drop_for_departed(c, Some("g"), "b", 60).unwrap();
    });
    assert!(fx.entries().is_empty());
    assert_eq!(fx.notices(), [LIMIT_NOTICE]);
}

#[test]
fn a_hand_off_dropped_because_the_group_has_no_lead_settles_its_parent_and_the_queue_goes_on() {
    let fx = Fixture::new();
    fx.add_cy();
    let first = fx.started("g", "hello");
    fx.finish_with(&first, "completed", "@Bo check");
    fx.db.with_connection(|c| {
        c.execute("UPDATE groups SET lead_agent_id = NULL WHERE id = 'g'", [])
            .unwrap();
        c.execute("DELETE FROM group_members WHERE agent_id = 'b'", [])
            .unwrap();
    });
    let launch = start_next_group(&fx.db, &fx.registry, &fx.dirs(), "g").unwrap();
    assert_eq!(launch.agent_id, "a");
    assert_eq!(
        fx.notices(),
        ["Bo did not finish; continuing without its answer."]
    );
    assert!(fx.entries().is_empty());
}
