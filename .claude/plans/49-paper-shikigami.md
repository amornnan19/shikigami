# Plan: issue #49, show subagents as paper shikigami

## Goal

When an agent starts subagents inside its turn (Claude Code's Agent/Task tool), show each one: a
nested group of its own steps under the step that started it, and a small paper figure (hitogata)
that folds out of the agent's avatar while it works and folds back when it is done. This fits the
product's name and gives the user a view of work that is hidden today.

## Decisions

### Data (Rust)

- Parser (`src-tauri/src/turns/claude_code.rs`): stream-json events carry `parent_tool_use_id`. A
  tool_use or tool_result inside a subagent has it set to the id of the subagent tool call. Today
  those inner events are skipped; instead parse them as steps with a `parent_step_id` = the step
  created for that tool_use id. Inner assistant text is not stored (the subagent's final result
  already arrives as the parent step's tool_result). Nested subagents (a subagent starting another)
  nest the same way.
- A step is a subagent step when its tool name is `Agent` or `Task`. Its label comes from the tool
  input: `subagent_type` (default "general-purpose") and `description` (fallback: the first 80
  characters of `prompt`). Keep these in the existing step input JSON; the view derives them.
- Migration V9 (follow the Database rules): `ALTER TABLE steps ADD COLUMN parent_step_id TEXT
  REFERENCES steps(id) ON DELETE CASCADE` plus an index on it.
- Inner steps go through exactly the same redaction, output bounds and step-count limits as other
  steps (they count toward the turn's limits).
- Turn events: the existing step events gain `parentStepId`; no new event kind.
- `TurnView` steps gain `parentStepId`; the UI builds the tree.

### UI (`src/features/chat/`, new `src/features/shikigami/`)

- Step list: a subagent step renders as a group: a header with a small paper figure icon, the
  subagent type and description, and a status (Working, Done, Failed); collapsible (open while
  running, collapsed when done); its inner steps inside, indented, using the existing step rows.
- Paper figure: an original SVG hitogata (a simple cut-paper human silhouette with a folded crease
  line, off-white paper with a thin ink outline, tinted with the agent's colour at low opacity). No
  imagery from existing franchises, no "shadow clone" styling. Drawn in code, no asset files.
- Motion, while the turn is running, next to the agent's avatar in the transcript (the turn header):
  - fold-out: the figure starts at the avatar, scale 0.2 → 1, rotateY 90° → 0, moves out to a slot
    beside the avatar, 450 ms ease-out;
  - working: opacity 0.75 and a slow 3 s float (±2 px);
  - done: folds back (reverse of fold-out, 350 ms) and disappears;
  - failed: tints red, then dissolves (opacity → 0, blur 2 px, 500 ms).
  - At most 3 figures beside an avatar; more show as "+N" on the last slot. Each subagent has its own
    figure. Figures carry the subagent type as a tooltip and `aria-label`.
- Reduce motion (`prefers-reduced-motion: reduce`): no figures animate; show a static stacked badge
  (two offset paper icons and the count of running subagents) beside the avatar instead.
- Group chat header: each member avatar shows a small badge with the number of paper figures that
  agent has out right now (running subagent steps of its running turn), hidden at 0.
- E2E-only switch `SHIKIGAMI_E2E_REDUCE_MOTION=1` (behind the `e2e` feature) makes the app report
  reduced motion, so the static badge can be screenshotted.

## Tests

- Rust: parser fixture with a subagent tool_use, inner tool_use/tool_result with
  `parent_tool_use_id`, and the final tool_result: steps and parents are right; a nested subagent;
  inner output redacted (a fake key in an inner tool result never reaches the DB or events); limits
  count inner steps; migration V9 on a V8 database keeps all steps.
- Vitest: tree building from flat steps; label fallback; figure state per status; reduced motion
  renders the badge and no animated figure.
- E2E (fake `claude` emitting two parallel subagents, one finishing and one failing, with a slow
  step so mid-run can be captured): screenshots of figures out mid-run, the nested groups after the
  turn, a failed subagent, the group header badge, and the reduced-motion badge.

## Out of scope

- Showing a subagent's intermediate assistant text. Sound.

## Done

`bun run check`; 10x `cargo test --lib`; `bun run e2e` 3/3; a design review of the screenshots.
