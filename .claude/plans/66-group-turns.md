# Plan: issue #66, run agent turns in a group chat

## Goal

A group chat has a shared transcript. A user message (no mentions yet; #67 adds them) goes to the
lead, whose turn runs Claude Code in the group folder with the agent's own instructions, a group
briefing and the messages it has not seen. Queue, Stop, stall, interruption and Resume behave as in
one-to-one chats.

## Decisions

### Data (migration V5, follow the Database rules in `CLAUDE.md`)

- `turns`: `ADD COLUMN group_id TEXT REFERENCES groups(id) ON DELETE CASCADE` (NULL = one-to-one).
- `messages`: `ADD COLUMN group_id TEXT REFERENCES groups(id) ON DELETE CASCADE` and
  `ADD COLUMN author_agent_id TEXT REFERENCES agents(id) ON DELETE SET NULL` (set on assistant and
  notice messages of group turns; NULL for the user).
- New `group_queue (id TEXT PRIMARY KEY, group_id TEXT NOT NULL REFERENCES groups(id) ON DELETE
  CASCADE, addressed_agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE, content TEXT
  NOT NULL, from_agent_id TEXT REFERENCES agents(id) ON DELETE SET NULL, root_message_id TEXT,
  hop INTEGER NOT NULL DEFAULT 0, position INTEGER NOT NULL, created_at INTEGER NOT NULL,
  UNIQUE (group_id, position))`. `from_agent_id`, `root_message_id` and `hop` are used by #67.
- New `agent_group_sessions (agent_id TEXT NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
  group_id TEXT NOT NULL REFERENCES groups(id) ON DELETE CASCADE, provider_session_id TEXT,
  last_seen_rowid INTEGER NOT NULL DEFAULT 0, PRIMARY KEY (agent_id, group_id))`.

### Turn rules (Rust, extend `src-tauri/src/turns/` with a group path; share the runner)

- One running turn per group (a running turn row with that `group_id`), and one running turn per
  agent across all chats (any running row with that `agent_id`). A group message whose addressed
  agent is busy elsewhere waits in the group queue; when that agent's other turn ends, the group
  queue is retried (the same advance hook as one-to-one queues).
- `send_group_message(groupId, text) -> { kind: "started" | "queued" }`: stores the user message in
  the group transcript (`group_id`, no author) and addresses the lead. Same validation as chats.
  Refused while the group has attention (`folder-missing`, `too-few-members`).
- A group turn runs the agent's provider in the group folder:
  - the agent's own `CLAUDE.md` text (from its workspace, up to 256 KiB) plus a group briefing are
    passed with `--append-system-prompt`. Briefing text:
    "You are <agent name>, a member of the group chat "<group name>". Members: <name> (<you|lead>),
    …. The user and other members write in this chat. To hand a task to another member, start a
    line with @Name followed by the task."
  - stdin carries the unseen group messages (rowid greater than the agent's `last_seen_rowid` in
    this group, oldest first, each as "<author>: <text>", author "User" or the agent's name), then a
    blank line, then the addressed message. After the turn starts, `last_seen_rowid` moves to the
    addressed message's rowid; after the turn ends, to the agent's own last message.
  - session per agent per group in `agent_group_sessions` (`--session-id` first, `--resume` after),
    with the same resume-failure rule as one-to-one.
  - isolation: the same flags as one-to-one, but the group folder's project settings must not load.
    Check whether the CLI accepts `--setting-sources ""` (read `claude --help`, then one throwaway
    run with model `claude-haiku-5-5` in subscription mode, in a temp folder that contains a
    `.claude/settings.json` with a distinctive permission allow rule and a `CLAUDE.md` with a
    distinctive line; the init line or a short prompt shows which loaded). Use the value that loads
    the folder's `CLAUDE.md` but not its settings. If no value does that, use `project` and report
    it (the decision is then logged in #54).
  - sign-in mode, environment allowlist, redaction, stall limits, Stop, shutdown rules: unchanged.
- Agent replies in groups are stored with `group_id` and `author_agent_id`.
- Startup sweep, interrupted turns, `resume_group_queue(groupId)`, attention: as one-to-one, scoped to
  the group.

### UI (`src/features/groups/`)

- Group transcript: each message shows the author's avatar and name (user messages right-aligned as
  in chats; agent messages left with the agent's avatar). Tool steps as in chats.
- Composer, queue list, Stop, Resume bar and stall/failed lines reuse the chat components with a
  group scope.
- Live updates through the existing `turn-event` (add `groupId` to its payload).

## Tests

- Rust: busy rules (group busy, agent busy elsewhere waits and then advances); the stdin of a group
  turn contains exactly the unseen messages in order and the addressed message; `last_seen_rowid`
  updates; a separate session per agent per group (one-to-one session untouched); the append
  system prompt carries the agent's instructions and the briefing; the chosen setting-sources value;
  migration V5 on a V4 database keeps everything; deleting a group cascades its messages and queue.
- E2E (fake `claude`): create a group with A (lead) and B, send "hello": A answers in the group
  transcript with its name; the fake records its cwd = group folder and its stdin; restart keeps the
  transcript; Stop and queue in the group; screenshots.

## Manual check

One real group turn with `claude-haiku-5-5` in subscription mode (`check-app`): A answers in the
group folder (ask it to list files; the listing shows the group folder's files). Plus the
setting-sources probe above (at most 2 extra raw runs).

## Done

`bun run check`; `bun run e2e` 3/3; spec checks 2, 7, 9, 10.
