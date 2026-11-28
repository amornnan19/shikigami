# Plan: issue #86, ask the user before risky actions with approval cards

## Goal

When an agent's CLI wants to do something that needs permission (a shell command, a web fetch, any
tool not allowed by the turn settings), the user sees a card in the chat and answers Allow once or
Deny. No answer means deny. Protected locations stay refused without a card.

## Decisions

### Transport (decide by probe first, then build)

- Preferred: Claude Code's documented `--permission-prompt-tool <mcp tool>` with
  `--permission-prompts host`. Shikigami provides the MCP server through `--mcp-config` (keep
  `--strict-mcp-config`, so it is the only server). The server is Shikigami's own executable started
  in a headless relay mode (`shikigami --approval-relay <socket path> <token>`, handled in `main`
  before Tauri starts, no window), speaking MCP over stdio and forwarding each permission call to a
  Unix socket owned by the running app in `<data>/tmp` (0600, per-turn token, removed after the turn
  and swept at startup). The relay never decides anything; the app answers.
- Read Anthropic's docs for the permission prompt tool's input and output shape (tool name, input,
  `behavior: allow|deny`, `updatedInput`, `message`) and confirm with one raw probe using a tiny
  throwaway relay before building.
- If the probe shows the MCP route cannot work, stop and report with evidence (the alternative is
  the stream-json control protocol, which needs a new decision).
- The relay's tool must not be callable by the model as a normal tool: deny it in the turn settings'
  permission rules for direct use if the CLI exposes it (check the init `tools` list in the probe;
  the canary check must still pass and must ignore this one MCP tool).

### What asks

- Shell commands (no Bash allow rule), WebFetch and WebSearch (`ask` rules from #85), and any other
  tool the CLI asks about. Reads outside the folder stay refused by
  `blockReadsOutsideWorkingDirectories` (stricter than the spec; update the spec line "Reading or
  listing anything outside the folder (outside protected locations)" to say it is refused for now).
- Allowed commands still run inside the sandbox (no change to #85 settings).

### Data (migration V10, Database rules)

`approvals (id TEXT PRIMARY KEY, turn_id TEXT NOT NULL REFERENCES turns(id) ON DELETE CASCADE,
tool TEXT NOT NULL, summary TEXT NOT NULL, detail TEXT NOT NULL, reason TEXT, status TEXT NOT NULL
CHECK (status IN ('pending','allowed','denied','timed-out')), answered_by TEXT CHECK (answered_by IN
('user','timeout','stop','shutdown')), created_at INTEGER NOT NULL, answered_at INTEGER)` + index on
turn_id. `summary` is the command or the tool and its target; `detail` the redacted input JSON
(bounded 4096 chars). Redact like step input.

### Flow (Rust, `src-tauri/src/approvals/`)

- A request creates a pending row and emits `approval-requested {approvalId, turnId, agentId,
  groupId}`; the relay call blocks until answered.
- `answer_approval(approvalId, allow: bool)`: only a pending row of a running turn; sets status and
  answered_by user; replies to the relay (`allow` with the original input, or `deny` with message
  "The user denied this request."); emits `approval-resolved`.
- Timeout 10 minutes (`SHIKIGAMI_APPROVAL_SECONDS` override in e2e builds only): timed-out, message
  "No answer within 10 minutes, so this request was denied."
- Stop, stall, turn end, shutdown (`kill_all`) and the relay connection dropping: every pending row
  of that turn becomes denied with answered_by stop/shutdown; the relay gets deny.
- The stall timer does not run while a card is pending (the turn is waiting for the user); it
  restarts when the card is answered.
- Startup sweep: pending rows become denied (shutdown).
- The agent's list row and the group's list row show "Needs you" while a card is pending (extend the
  existing attention/state shape; the sidebar dot uses the amber attention style with the label
  "Waiting for you").

### UI

- `ApprovalCard` in the transcript under the running turn (and in group transcripts): agent avatar
  and name, "wants to run" + monospace summary, "in <folder>" (folder basename with full path in a
  tooltip), the reason if any, buttons **Allow once** and **Deny**; after an answer the buttons are
  replaced by the outcome ("Allowed", "Denied", "Timed out", "Denied because the turn stopped") and
  the time. Keyboard reachable; no auto-focus stealing from the composer.
- Past cards render from stored rows after restart.

## Tests

- Rust: relay protocol round-trip with a fake client; allow, deny, timeout (short override), stop
  and shutdown denials; only pending rows of running turns can be answered; stall paused while
  pending; migration V10 on a V9 database; redaction of detail.
- E2E (fake claude that calls the relay like the real CLI would): a command card; Allow once makes
  the fake run it; Deny is reported; timeout with a short override; Stop with a card open; restart
  shows outcomes; a group card. Screenshots of each state.

## Manual check

One real turn with `claude-haiku-5-5` in subscription mode: "run `ls -la` in your folder" shows a
card; Allow once runs it inside the sandbox; a second request "run `curl -sS https://example.com`"
allowed still fails (no network). Screenshots.

## Done

`bun run check`; 10x `cargo test --lib`; `bun run e2e` 3/3; the manual check.
