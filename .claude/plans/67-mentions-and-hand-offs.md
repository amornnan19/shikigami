# Plan: issue #67, mention agents and hand work on in a group chat

## Goal

The user addresses group members with `@Name`, and agents hand tasks to each other by starting a
line with `@Name` in their reply. The agent that handed off continues with the answer. Limits stop
runaway chains.

## Decisions

### Mention parsing (Rust, `src-tauri/src/groups/mentions.rs`, pure functions)

- A mention is `@` followed by a member's name, matched case-insensitively, choosing the longest
  member name that matches at that point (names may contain spaces), followed by the end of the
  text, whitespace, or one of `, . : ; ! ? ) ]`. Text inside fenced code blocks (```` ``` ````) and
  inline code (`` `…` ``) is ignored. A non-member `@word` is plain text.
- User message: the addressed members are the mentioned members in order of first appearance,
  without duplicates. No member mentioned: the lead.
- Agent reply (final assistant text of a finished turn): a hand-off line is a line whose first
  non-space characters are a mention of a member other than the replying agent. The task is the rest
  of that line plus the following lines up to the next hand-off line or the end. Lines before the
  first hand-off line are the agent's own answer. Several hand-off lines give several hand-offs, in
  order.

### Flow (Rust, group queue from #66)

- A user message with several addressed members queues one entry per member, in order, each with
  `root_message_id` = the user message and `hop` = 0.
- When an agent's turn finishes (`completed` only; not stopped, failed or interrupted), each
  hand-off becomes a queue entry for the target with `from_agent_id` = the replying agent, the task
  text as `content`, the same `root_message_id`, `hop` = parent hop + 1. Hand-off entries are placed
  before any later user-originated entries of the same root (they continue the current chain).
- When a turn that came from a hand-off completes, a continuation entry is queued for the agent
  that handed off: content "Continue with <member>'s answer." (the answer reaches it as an unseen
  message), same root, next hop. If that agent's own continuation reply hands off again, the chain
  continues.
- Limits per `root_message_id`:
  - at most 8 agent turns in total; when the next entry would be the 9th, it and the remaining
    agent-originated entries of that root are removed, and a group notice is posted: "Hand-off limit
    reached. Mention an agent to continue.";
  - ping-pong: if the hand-off would be the third consecutive hand-off between the same two agents
    (A→B, B→A, A→B counts as three), it is not queued; notice: "<A> and <B> keep handing work back;
    that hand-off was not started."
- Stop on a running group turn: the turn ends stopped, its reply is not parsed for hand-offs, and no
  continuation is queued for it. Entries already in the queue stay (the user can remove them).
- Permissions: `access_for_hand_off(from, to) -> AccessLevel` returns the lower of the two agents'
  levels; every agent currently has the single level `standard`, so it always returns `standard`.
  The group launch path takes the access level from it, ready for M2. Unit-tested with a fake second
  level.

### UI

- Composer: typing `@` opens a small suggestion list of members (avatar and name) filtered by the
  typed text; arrow keys move, Enter or Tab inserts `@Name ` (with the name's real casing), Escape
  closes. Mouse works too.
- Rendering: mentions of members inside messages are shown as a coloured name chip (the member's
  colour). Hand-off lines in an agent's reply get a small "→ Handed to <Name>" marker. Limit and
  ping-pong notices render as muted centred lines.

### Fit with what #66 built (read `src-tauri/src/turns/{mod,group,runner}.rs` first)

- Reuse `group_queue` (`from_agent_id`, `root_message_id`, `hop` already exist) and the existing
  begin/queue/advance path, busy rules, `Chain`, `RunEnd` and the `TurnRegistry` agent hold. A
  hand-off or continuation entry addressed to a held agent waits like any other entry.
- "Hand-off entries are placed before later user-originated entries of the same root": implement by
  inserting at the position right after the last entry of the same root (shift later positions in
  the same transaction), keeping `UNIQUE (group_id, position)` valid.
- The 8-turn count per root counts group turns already run for that root (store `root_message_id`
  on the group turn row if needed: migration V8, add column, Database rules) plus the one about to
  start.
- Hand-offs are parsed from the stored final assistant text of a turn that ended `completed` (after
  redaction). A turn ending stopped, failed, interrupted or stalled produces no hand-off and no
  continuation.
- The hand-off rule text already sent in the group details must match these parsing rules; update
  its wording if needed (that changes the context hash, which is fine).
- User mentions are resolved inside the same transaction as the send (as the lead is today).
- Mentions match member names only; a mention of a departed member at advance time re-addresses to
  the lead like other entries.

## Tests

- Rust, parsing: case-insensitive, longest match for names with spaces ("@Test Agent" vs "@Test"),
  punctuation boundaries, code fences and inline code ignored, non-members ignored, multiple
  mentions in order and deduplicated, hand-off line extraction with multi-line tasks, self-mention
  ignored.
- Rust, flow (fake claude): user "@B list the files" → B's turn; A's reply with "@B check the README"
  → B runs next, then A gets a continuation; 8-turn limit posts the notice and removes the rest;
  ping-pong guard; stop prevents hand-offs and continuation; failed turns do not hand off; several
  user mentions run in order.
- E2E (fake claude scripted by prompt content): spec checks 3-6, screenshots of the suggestion list,
  chips, hand-off marker and limit notice.

## Done

`bun run check`; `bun run e2e` 3/3; spec checks 3, 4, 5, 6.
