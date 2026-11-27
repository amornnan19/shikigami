# Plan: issue #16, create and list agents

## Goal

The user creates an agent (name, colour, provider, model) and sees it in a sidebar. Creating an
agent makes its workspace folder and instruction file. Agents survive restarts. This is the shell
every later M1 issue fills in.

## Decisions

### Rust

- New module `src-tauri/src/agents/` with `mod.rs` (types, commands), `store.rs` (database access),
  `workspace.rs` (folder name and folder creation), `validate.rs`.
- Workspaces root: `~/Shikigami/agents`. The environment variable `SHIKIGAMI_WORKSPACES_DIR`
  (absolute path only, like `SHIKIGAMI_DATA_DIR`) overrides it, for tests and e2e. Resolve it once at
  startup into managed state `WorkspacesRoot(PathBuf)`; if HOME cannot be found and no override is
  set, the create command returns an error (startup still works).
- `Agent` (serde, camelCase): `id`, `name`, `color`, `provider`, `model`, `workspacePath`,
  `createdAt`, `lastActivityAt`, `state`.
- `state` is derived when listing, never stored: `working` if the agent has a turn with status
  `running`; `needs-attention` if its newest turn is `failed` or `interrupted`, or its workspace
  folder does not exist, or its provider is not installed; otherwise `idle`. In this issue no turns
  exist yet, so the turn rules are written and tested with rows inserted by the test.
- Colour palette (stored as the name): `slate`, `red`, `orange`, `amber`, `green`, `teal`, `blue`,
  `violet`, `pink`. The UI maps names to Tailwind classes.
- Models: store the string as given. `default` means "pass no model flag". Suggested lists for the
  UI, sent by a `model_suggestions` part of the command response or a constant in the UI (UI
  constant is fine): Claude Code: `default`, `opus`, `sonnet`, `haiku`. Codex: `default`,
  `gpt-5-codex`, `gpt-5`. Free text allowed, trimmed, 1-100 characters, no whitespace inside.
- Validation (Rust is the authority; the UI mirrors it for instant feedback):
  - name trimmed, 1-40 characters (count Unicode scalar values), unique case-insensitively
    (the column already has `COLLATE NOCASE UNIQUE`); error codes `name-empty`, `name-too-long`,
    `name-taken`.
  - colour in the palette: `color-invalid`.
  - provider is `claude-code` or `codex` and currently installed (use the providers module):
    `provider-unavailable`.
  - model as above: `model-invalid`.
  - Errors cross IPC as `{ code, message }` so the UI can place them on the right field.
- Folder name from the agent name: Unicode-lowercase, replace runs of whitespace and `_` with `-`,
  keep only `a-z`, `0-9` and `-`, collapse repeated `-`, trim `-`, cut to 40 characters; empty
  result becomes `agent`. If the folder exists, append `-2`, `-3`, ... until a free name is found.
  Create it with `create_dir` (not `create_dir_all` for the last part) so a race cannot reuse an
  existing folder. Create the root with `create_dir_all`.
- Instruction file: `CLAUDE.md` for Claude Code, `AGENTS.md` for Codex (each CLI's own published
  convention). Content:

  ```
  # <name>

  You are <name>, a coding agent working in this folder.
  ```

- Create order: validate, create folder, write the instruction file, insert the row. If writing
  the file or inserting the row fails, remove the folder that was just created (only that folder,
  only if this call created it) and return the error. No agent row may exist without its folder.
- Commands: `list_agents() -> Vec<Agent>` ordered by `last_activity_at` desc, then `created_at`
  desc; `create_agent(input) -> Agent`. Both use the `Database` state; if the app started with an
  error they return an error.
- `created_at` and `last_activity_at` are set to now on create.

### UI

- Layout when the app started ok: a left sidebar (240 px) and a main area.
  - Sidebar: app name at the top, a "New agent" button, then the agent list. Each row: a round
    avatar with the first letter of the name on the agent's colour, the name, and a small state
    dot (idle: none, working: animated dot, needs attention: amber dot with an accessible label).
    The selected row is highlighted.
  - Main area with no agents: the existing welcome text and the Providers section.
  - Main area with an agent selected: a header with avatar and name, and an empty chat placeholder
    ("No messages yet."). The chat itself is #17.
- New agent dialog (shadcn `dialog`, `input`, `label`, `select`; add them by hand or with the
  shadcn CLI, exact-pinned Radix deps):
  - Name input, colour swatches (radio group), provider select (unavailable providers disabled with
    the reason, e.g. "Not installed"), model combobox: a select of suggestions plus an "Other..."
    option that shows a text input.
  - Create disabled until the form is valid. Server errors appear under the field they belong to.
  - On success: close, refresh the list (TanStack Query invalidation), select the new agent.
- Data through TanStack Query: `agents` query and `createAgent` mutation in
  `src/features/agents/api.ts`. Components in `src/features/agents/`.
- Interface text in English, as literals for now (no i18n layer yet).

## Tests

- Rust:
  - folder name derivation: spaces, punctuation, Thai or emoji only (becomes `agent`), length cut,
    collisions get `-2`, `-3`.
  - validation codes for each rule, including a case-insensitive duplicate.
  - create makes the folder and the right instruction file for each provider.
  - a forced failure after the folder is made (for example the database insert fails because of a
    duplicate inserted concurrently by the test) removes the folder and leaves no row.
  - state derivation: no turns idle, running turn working, failed newest turn needs attention,
    missing folder needs attention, provider not installed needs attention.
  - list order by last activity.
- E2E (fake `claude` provider via `SHIKIGAMI_PROVIDER_PATH`, temp `SHIKIGAMI_DATA_DIR` and
  `SHIKIGAMI_WORKSPACES_DIR`):
  - create "Test Agent" with Claude Code: it appears in the sidebar and is selected; the folder
    `test-agent` and its `CLAUDE.md` exist (check from the test process).
  - creating "test agent" again shows the name-taken error under the name field.
  - Codex shows as unavailable in the provider select (no fake codex).
  - screenshots of the dialog, the error, and the list.

## Out of scope

Chat (#17), edit and delete (#20), provider switching.

## Done

`bun run check` and `bun run e2e` pass; the new e2e spec passes 3/3; a manual run of the real app
creates an agent under a temp workspaces dir and shows it after restart.
