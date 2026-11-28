use serde_json::Value;
use tokio::time::Instant;

use super::Runner;
use crate::approvals::store::{
    self, NewApproval, Outcome, ALLOWED_BY_USER, DENIED_BY_RELAY, DENIED_BY_SHUTDOWN,
    DENIED_BY_STOP, DENIED_BY_USER, TIMED_OUT,
};
use crate::approvals::{
    allow_reply, deny_reply, Channel, DENY_BUSY, DENY_INTERRUPTED, DENY_SHUTDOWN, DENY_STOP,
    DENY_TIMEOUT, DENY_UNREACHABLE, DENY_USER, MAX_OPEN,
};
use crate::turns::claude_code::{truncate, DETAIL_LIMIT};
use crate::turns::redact::Redactor;
use crate::turns::TurnEvent;

const SUMMARY_LIMIT: usize = DETAIL_LIMIT;
const REASON_LIMIT: usize = 400;
const TARGET_FIELDS: [&str; 6] = ["command", "file_path", "path", "url", "pattern", "query"];
const LOCATION_FIELDS: [&str; 3] = ["file_path", "path", "url"];

pub(super) struct Pending {
    conn: u64,
    request: u64,
    input: Value,
    deadline: Instant,
}

pub(super) struct Described {
    summary: String,
    detail: String,
    reason: Option<String>,
    target: Option<String>,
}

fn text_field<'a>(input: &'a Value, name: &str) -> Option<&'a str> {
    input
        .get(name)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
}

pub(super) fn describe(redactor: &Redactor, tool: &str, input: &Value) -> Described {
    let target = TARGET_FIELDS
        .iter()
        .find_map(|field| text_field(input, field));
    let summary = match (tool, target) {
        ("Bash", Some(command)) => command.to_owned(),
        (_, Some(target)) => format!("{tool} {target}"),
        _ => tool.to_owned(),
    };
    let redacted = redactor.redact_value(input);
    let detail = serde_json::to_string_pretty(&redacted).unwrap_or_default();
    Described {
        summary: truncate(&redactor.redact(&summary), SUMMARY_LIMIT),
        detail: truncate(&detail, DETAIL_LIMIT),
        target: LOCATION_FIELDS
            .iter()
            .find_map(|field| text_field(input, field))
            .map(|target| truncate(&redactor.redact(target), SUMMARY_LIMIT)),
        reason: text_field(input, "description")
            .map(|reason| truncate(&redactor.redact(reason), REASON_LIMIT)),
    }
}

#[cfg(test)]
impl Described {
    pub(super) fn summary(&self) -> &str {
        &self.summary
    }

    pub(super) fn target(&self) -> Option<&str> {
        self.target.as_deref()
    }

    pub(super) fn reason(&self) -> Option<&str> {
        self.reason.as_deref()
    }
}

impl Runner<'_> {
    pub(super) fn next_approval_deadline(&self) -> Option<Instant> {
        self.pending.values().map(|pending| pending.deadline).min()
    }

    pub(super) async fn open_approval(
        &mut self,
        session: &Channel,
        conn: u64,
        request: u64,
        tool: String,
        input: Value,
    ) {
        if self.registry.is_closing() {
            session
                .reply(conn, request, deny_reply(DENY_SHUTDOWN))
                .await;
            return;
        }
        if self.registry.stop_requested(&self.agent_id, &self.turn_id) {
            session.reply(conn, request, deny_reply(DENY_STOP)).await;
            return;
        }
        if self.pending.len() >= MAX_OPEN {
            session.reply(conn, request, deny_reply(DENY_BUSY)).await;
            return;
        }
        let described = describe(&self.redactor, &tool, &input);
        let id = uuid::Uuid::now_v7().to_string();
        let created_at = self.tick();
        let turn_id = self.turn_id.clone();
        let saved = self.db.with_connection(|connection| {
            store::insert(
                connection,
                &NewApproval {
                    id: &id,
                    turn_id: &turn_id,
                    tool: &tool,
                    summary: &described.summary,
                    detail: &described.detail,
                    reason: described.reason.as_deref(),
                    target: described.target.as_deref(),
                    created_at,
                },
            )?;
            store::view(connection, &id)
        });
        let Ok(Some(view)) = saved else {
            session
                .reply(conn, request, deny_reply(DENY_UNREACHABLE))
                .await;
            return;
        };
        self.pending.insert(
            id,
            Pending {
                conn,
                request,
                input,
                deadline: Instant::now() + self.approval_timeout,
            },
        );
        self.emit(TurnEvent::ApprovalRequested { approval: view });
    }

    async fn decide(&mut self, session: &Channel, approval_id: &str, outcome: Outcome) {
        let Some(pending) = self.pending.remove(approval_id) else {
            return;
        };
        let now = self.tick();
        let recorded = self
            .db
            .with_connection(|connection| store::resolve(connection, approval_id, &outcome, now))
            .unwrap_or(false);
        let reply = match (recorded, outcome.status) {
            (true, "allowed") => allow_reply(&pending.input),
            (true, _) => deny_reply(denial_message(&outcome)),
            (false, _) => deny_reply(DENY_UNREACHABLE),
        };
        session.reply(pending.conn, pending.request, reply).await;
        if recorded {
            if let Ok(Some(view)) = self
                .db
                .with_connection(|connection| store::view(connection, approval_id))
            {
                self.emit(TurnEvent::ApprovalResolved { approval: view });
            }
        }
    }

    pub(super) async fn answer_approval(
        &mut self,
        session: &Channel,
        approval_id: &str,
        allow: bool,
    ) {
        let outcome = if allow {
            ALLOWED_BY_USER
        } else {
            DENIED_BY_USER
        };
        self.decide(session, approval_id, outcome).await;
    }

    pub(super) async fn expire_approvals(&mut self, session: &Channel, now: Instant) {
        let due: Vec<String> = self
            .pending
            .iter()
            .filter(|(_, pending)| pending.deadline <= now)
            .map(|(id, _)| id.clone())
            .collect();
        for id in due {
            self.decide(session, &id, TIMED_OUT).await;
        }
    }

    pub(super) async fn deny_approvals(&mut self, session: &Channel, closing: bool) {
        let ids: Vec<String> = self.pending.keys().cloned().collect();
        for id in ids {
            let outcome = if closing {
                DENIED_BY_SHUTDOWN
            } else {
                DENIED_BY_STOP
            };
            self.decide(session, &id, outcome).await;
        }
    }

    pub(super) async fn drop_connection(&mut self, session: &Channel, conn: u64) {
        let ids: Vec<String> = self
            .pending
            .iter()
            .filter(|(_, pending)| pending.conn == conn)
            .map(|(id, _)| id.clone())
            .collect();
        for id in ids {
            self.decide(session, &id, DENIED_BY_RELAY).await;
        }
    }
}

fn denial_message(outcome: &Outcome) -> &'static str {
    match outcome.answered_by {
        "timeout" => DENY_TIMEOUT,
        "stop" => DENY_STOP,
        "shutdown" => DENY_SHUTDOWN,
        "relay" => DENY_INTERRUPTED,
        _ => DENY_USER,
    }
}
