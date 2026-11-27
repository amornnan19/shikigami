# Plan: issue #20, edit and delete agents

## Goal

The user can change an agent's name, colour, model and sign-in mode, edit its instruction file,
and delete it, without losing the conversation (edits) and without destroying files by surprise
(delete).

## Decisions

### Rust (`src-tauri/src/agents/`)

- `update_agent(agentId, { name?, color?, model? }) -> Agent`: same validation as create; the name
  must be unique among other agents (case-insensitive). The workspace folder is never renamed or
  moved. Takes effect from the next turn.
- Sign-in override already exists as `set_agent_auth_mode` (#50); the settings UI uses it.
- `read_instructions(agentId) -> { text, modifiedAt }` reads `<workspace>/CLAUDE.md`. Missing file:
  `{ text: "", modifiedAt: null }`. Files over 256 KiB: error `too-large`.
- `write_instructions(agentId, text, expectedModifiedAt, force) -> { modifiedAt }`:
  - refuses if the path is a symlink or not a regular file (`not-a-file`);
  - if the file's current mtime differs from `expectedModifiedAt` and `force` is false, error
    `changed-on-disk` and write nothing;
  - writes atomically: a temp file in the same folder, then rename over `CLAUDE.md`;
  - text over 256 KiB: `too-large`.
- `reveal_workspace(agentId)`: shows the workspace folder in Finder through the opener plugin's
  reveal function, using the path from the database (the UI never passes a path).
- `delete_agent(agentId, moveFolderToTrash) -> ()`:
  - refused with `busy` ("Stop the agent before deleting it.") while a turn runs;
  - deletes the agent row in one transaction (turns, messages, steps and queue cascade);
  - with `moveFolderToTrash`: moves the recorded workspace folder to the macOS Trash with the
    `trash` crate (check its license), only if the path is an existing directory (not a symlink)
    inside the workspaces root after canonicalising both; otherwise the folder is left and the
    result says so (`folderKept: true` with a reason). The row is deleted first; a Trash failure
    never restores the row and is reported.
  - Under the `e2e` feature only, `SHIKIGAMI_E2E_TRASH_DIR` replaces the Trash with a plain move into
    that folder, so tests never fill the user's Trash.

### UI

- A gear button in the chat header opens "Agent settings" (shadcn dialog, wide):
  - Identity: name input, colour swatches.
  - Model: the same select-plus-other control as the create dialog.
  - Sign-in: "Use app default (…)", "Claude subscription", "API key".
  - Instructions: a monospace textarea with the file text, Save, and "Show in Finder". On
    `changed-on-disk`: "CLAUDE.md changed on disk since you opened it. Save again to overwrite it."
    and the next Save passes `force`.
  - Delete: a "Delete agent" button opening a confirmation: "Delete <name>? Its conversation is
    removed from Shikigami." with a checkbox "Also move the folder <path> to the Trash" (off by
    default). Disabled with a hint while the agent is working.
- Save buttons per section; errors under their fields; queries invalidated after each change.

## Tests

- Rust: update validation (duplicate excluding self, same name allowed); history intact after
  update; folder not renamed; instructions read/write, mtime conflict, force, symlink refusal,
  atomic write leaves no temp file; delete refused while running; delete cascades; trash only
  inside the root, symlink and outside paths kept; e2e trash override compiled only with the
  feature.
- E2E: rename and recolour (sidebar updates, folder unchanged), change model and see the header;
  edit instructions and save; change the file from the test process then save → warning, save again
  overwrites; delete without the checkbox (folder stays) and with it (folder moved to the e2e trash
  dir); delete refused while a slow turn runs. Screenshots.

## Manual check

None with the real CLI.

## Out of scope

Changing the workspace folder, duplicating agents.

## Done

`bun run check`; `bun run e2e` 3/3; spec checks 8, 9 and 11.
