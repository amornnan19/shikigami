# Plan: issue #17, chat with a Claude Code agent

## Goal

The user sends a message to a Claude Code agent and watches the reply stream in, with tool steps
shown compactly. Every message, reply and step is saved as it arrives, and the agent keeps its
Claude Code session across messages and restarts. This is the first real turn runner; stop and
queue (#18), recovery and stalls (#19) and Codex (#21) build on it.

## Decisions

### Running a turn (Rust, new module `src-tauri/src/turns/`)

Files: `mod.rs` (commands, events, registry), `claude_code.rs` (command line and stream parsing),
`runner.rs` (process lifecycle), `store.rs` (database writes and reads).

- One turn = one CLI process. Rejected: a long-lived interactive process per agent (harder to
  recover after a crash, and stop/queue become simpler with one process per turn).
- Command line, built with an argument vector (no shell), absolute path from the providers search:
  `claude -p --output-format stream-json --verbose --include-partial-messages`
  plus `--model <model>` unless the agent's model is `default`,
  plus `--session-id <uuid>` on the agent's first turn (a new UUID v4 that Shikigami generates and
  stores before starting) or `--resume <provider_session_id>` on later turns.
  The user's message is written to the process's stdin, then stdin is closed. Rejected: the message
  as an argument (visible in `ps`, length limits).
- Never pass any flag that skips or widens permissions (`--dangerously-skip-permissions`,
  `--permission-mode` other than the default, `--allowedTools`). In print mode Claude Code denies
  tools that would need approval; such a step shows as failed with the CLI's own message. Approvals
  arrive in M2.
- Working directory: the agent's workspace folder. If it does not exist, the turn fails at once
  with "The workspace folder is missing." and no process starts.
- Environment: follow "Running agent CLIs" in `CLAUDE.md`: explicit `PATH` from the provider search
  list, own process group, bounded reads. Secrets: call `api_key::api_key_for_turn` on
  `spawn_blocking`; when it returns a key, set `ANTHROPIC_API_KEY`; when it returns `None`, remove
  `ANTHROPIC_API_KEY` and `ANTHROPIC_AUTH_TOKEN` from the child environment; when it errors, fail the
  turn with "Could not read the saved API key." and start no process.
- Stream parsing: read stdout line by line (each line JSON, lines over 1 MiB are an error that fails
  the turn). Use only these message kinds, ignoring unknown ones:
  - `system` with `subtype: "init"`: take `session_id`; store it as the agent's
    `provider_session_id` if it differs.
  - `stream_event` partial deltas of text content: append to the assistant text and emit a
    `text-delta` event.
  - `assistant` messages: their `tool_use` blocks start steps (one step per tool use id); their text
    blocks are the authoritative text for that message (replace the streamed text for that block).
  - `user` messages with `tool_result` blocks: end the matching step (`done`, or `failed` when
    `is_error`).
  - `result`: ends the turn. `is_error: false` gives `completed`; `is_error: true` gives `failed`
    with the result text as the error.
  The implementer must confirm the exact JSON shapes by capturing one real run (see Manual check)
  and save that capture, with any absolute home paths replaced by `/Users/test`, as the fixture
  `src-tauri/tests/fixtures/claude-stream-basic.jsonl` (and one with a tool use,
  `claude-stream-tool.jsonl`). The parser is written against those fixtures.
- Exit handling: if the process exits without a `result` line, the turn is `failed` with the last
  stderr line (bounded to 4 KiB, never containing the environment) or "Claude Code exited
  unexpectedly."
- Session resume failure: if a `--resume` run exits before an `init` line with an error, clear the
  stored session id, add a notice message "Started a new Claude Code session. The agent may have
  lost earlier context.", and retry the same turn once with a new `--session-id`.
- Concurrency: one running turn per agent. `send_message` while a turn runs returns error `busy`
  (the queue is #18). A `TurnRegistry` (managed state, `Mutex<HashMap<agent_id, RunningTurn>>`)
  holds the running turn's process group id for #18's Stop.
- Startup sweep: on launch, any turn still `running` in the database becomes `interrupted`
  (full recovery UI is #19).

### Storage (no new migration; V1 tables)

- `send_message` in one transaction: insert the turn (`running`) and the user message
  (`role = user`, `turn_id` set), update the agent's `last_activity_at`.
- The assistant reply is one `messages` row per assistant text block in order (`role = assistant`).
  Streamed text is written to the database at most every 500 ms and at block end, not per delta.
- Steps: `kind` = tool name, `title` = a one-line summary built from the tool input (for example the
  `file_path`, `pattern`, `command` or `url` field when present, else the tool name), `detail` = the
  tool input JSON and the result text, each truncated to 4 KiB with "…" appended, `position` in
  order of appearance.
- Turn end: status, `ended_at`, `error`; agent `last_activity_at`.

### Commands and events

- `send_message(agentId, text) -> { turnId }`. Errors `{ code, message }`: `empty`, `too-long`
  (over 100,000 characters), `busy`, `provider-unavailable`, `not-found`, `database-failed`.
- `get_conversation(agentId, beforeTurnId?) -> { turns: TurnView[], hasMore }`, newest 20 turns
  before the cursor, returned oldest first. `TurnView { id, status, error, startedAt, endedAt,
  userMessage, items: (AssistantText { id, text } | Step { id, kind, title, detail, status })[] }`
  in order. Notices are returned as `items` of kind `notice`.
- Tauri event `turn-event`, payload `{ agentId, turnId, kind, ... }` with kinds `started`,
  `text-delta { itemId, text }`, `text-set { itemId, text }`, `step-started { step }`,
  `step-ended { stepId, status, detail }`, `notice { itemId, text }`, `ended { status, error }`.
- `list_agents` state already derives `working` from running turns.

### UI (`src/features/chat/`)

- `AgentView` becomes the chat: header (avatar, name, model), a scrollable transcript, and the
  composer at the bottom.
- Transcript: each turn shows the user message (right aligned bubble), then the agent's items:
  - assistant text rendered as Markdown with `react-markdown` and `remark-gfm` (exact pins, MIT);
    code blocks with a Copy button; links open in the default browser through the opener plugin;
  - steps collapsed into one line per turn: "N steps" with a chevron; expanded, each step shows
    an icon by status, its title, and its detail in a monospace block;
  - notices as a muted centered line;
  - a failed turn shows its error in a muted red line below.
- Live updates: listen to `turn-event` and patch the TanStack cache for that agent's conversation;
  refetch on `ended`.
- Composer: a textarea (Enter sends, Shift+Enter new line), Send button, disabled while empty, while
  a turn runs (#18 adds the queue), and over 100,000 characters (with the count shown).
- Opening an agent scrolls to the newest message. "Load older messages" button at the top when
  `hasMore`.
- Opener capability: allow `https://*` and `http://*` URLs (agent links can go anywhere on the web)
  in addition to the install links; still no `mailto`, `file` or custom schemes.

### Tests

- Rust, parser against the fixtures: text deltas assemble into the final text; tool use creates a
  step with the expected title and ends `done`; an error tool result ends `failed`; `result`
  ends the turn; unknown message kinds are ignored; a line over 1 MiB fails the turn.
- Rust, runner with a fake `claude` script (in a temp search dir) that prints a fixture and checks
  its own arguments and stdin:
  - first turn passes `--session-id`, later turns `--resume` with the stored id;
  - `default` model passes no `--model`;
  - the message arrives on stdin, not in argv;
  - with a saved and enabled key (memory store) the child sees `ANTHROPIC_API_KEY`; with the switch
    off the child does not see it even if the parent environment has it;
  - no permission-widening flag is ever passed;
  - a resume failure retries once with a new session and adds the notice;
  - process exit without `result` gives `failed` with the stderr line;
  - a missing workspace fails without starting the process.
- Rust, store: send inserts turn and user message atomically; `busy` when a turn runs; the startup
  sweep turns `running` into `interrupted`; `get_conversation` pages by 20 turns.
- E2E (fake `claude` that replays the fixtures): send "Reply with the word pong" and see the
  streamed reply; a tool-use fixture shows "1 step" collapsed, expandable to the title; reload the
  app (restart via a second wdio config sharing the same data dir, or reopen the agent) and the
  conversation is still there; screenshots of each state.

## Manual check

Use `check-app` with temp data and workspace folders and the real `claude` CLI. The saved Anthropic
API key is in the real Keychain, so this check uses it automatically (do not print or read it).
Create an agent with model `claude-haiku-5-5`. Keep real turns to at most 4:
1. "Reply with the word pong" streams a reply containing "pong" (spec check 2).
2. "List the files in your folder" shows a tool step and an answer (spec check 3).
3. A follow-up "What was my first message?" shows the session is kept.
4. Quit and reopen with the same data dir: the conversation is there (spec check 6).
Capture the stream of turn 1 and turn 2 for the fixtures (redact home paths). Screenshots of each.

## Out of scope

Stop and queue (#18), recovery UI, stall timeout and needs-attention reasons (#19), editing agents
(#20), Codex (#21), attachments, multiple threads.

## Done

`bun run check` and `bun run e2e` (3/3) pass; the manual check passes with Haiku 5.5.

## Review fixes (round 2)

Decided with the user on 2026-11-28: agents are fully isolated from the user's own Claude Code
configuration, and the CLI's own sign-in keeps working.

1. Isolation flags on every turn: `--setting-sources project --strict-mcp-config
   --permission-mode manual --permission-prompts none`. Keep `--model`, `--session-id` / `--resume`.
   Tests: replace the "no argument contains permission" checks with a check that the forbidden values
   never appear (`bypassPermissions`, `acceptEdits`, `auto`, `dontAsk`, any `--dangerously-*`,
   `--allowedTools`, `--setting-sources` containing `user`) and that the four isolation flags always
   do. Update the e2e fake and spec the same way.
2. Redact the saved key (and any `sk-ant-` token of 20+ characters from the key alphabet) from every
   text before it is saved or emitted: step input, step output, text deltas (redact the stored text;
   hold back the last 120 characters of a streaming block until the block ends so a key split across
   deltas is never emitted whole), `text-set`, notices and errors. Test: a fake tool result containing
   the key is stored and emitted as `[redacted]`.
3. On app exit (`RunEvent::Exit` / `ExitRequested`), kill every registered process group: SIGTERM,
   wait up to 2 s, then SIGKILL, synchronously. Test the registry function that does it.
4. Resume failure only when stderr or the result text contains "No conversation found". Do not store
   the fresh session id before the retry; the init line stores it. If a `--session-id` attempt never
   sees init, set `provider_session_id` back to NULL. Tests for both cases.
5. Markdown: do not render images (`disallowedElements={["img"]}`, show the alt text and the URL as
   a plain link instead). Add a Content Security Policy in `tauri.conf.json`: `default-src 'self';
   img-src 'self' data:; style-src 'self' 'unsafe-inline'; connect-src 'self' ipc: http://ipc.localhost`
   (adjust only what Tauri 2 needs; read the Tauri CSP docs). E2E must still pass.
6. Refetch during a stream: in the conversation `queryFn`, read the cache after the `await` and, for a
   turn still running, keep the longer text per item id. De-duplicate notices by id.
7. With a key present, also remove `ANTHROPIC_AUTH_TOKEN` and `ANTHROPIC_BASE_URL` from the child
   environment. Abort the stderr task when the turn ends. Replace `env::set_var` in tests with an
   injected environment.
8. Keychain: in `key_for_turn`, if `exists` is true and `get` returns `None`, return an error (the turn
   fails with "Could not read the saved API key."). Record the raw OSStatus code (never the value) in
   the error kept for diagnostics. Test with a store whose `exists` is true and `get` is `Ok(None)`,
   through the same mapping `drive` uses.
9. `CLAUDE.md`: in "Running agent CLIs" add: "Start each agent CLI isolated from the user's own
   configuration of that CLI: no user-level settings, hooks, plugins, output styles or MCP servers.
   For Claude Code: `--setting-sources project --strict-mcp-config --permission-mode manual
   --permission-prompts none`. Keep the CLI's own sign-in working." In "Secrets" add: "Redact a secret
   passed to a child from every text that reaches the database or an event: step input and output,
   reply text, notices and errors."

Manual check (round 2): 2 real turns with `claude-haiku-5-5` through the app, using the saved key.
The user must approve the macOS Keychain prompt; never click it yourself. Capture the init line and
confirm `permissionMode` is `manual`, there is no output style, no MCP servers or plugins, and
`apiKeySource` shows the key was used. Turn 1 "Reply with the word pong"; turn 2 "List the files in
your folder" (expect a read-only tool such as LS or Glob to work, or Bash to be denied with the CLI's
message). Re-capture the fixtures from these runs (redacted).
