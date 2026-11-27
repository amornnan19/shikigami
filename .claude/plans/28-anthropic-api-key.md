# Plan: issue #28, use an Anthropic API key for Claude Code agents

## Goal

The user saves an Anthropic API key in Shikigami. Claude Code turns can then authenticate with it
instead of the CLI's own sign-in. The key lives only in the macOS Keychain and never comes back to
the UI.

## Decisions

### Secret storage

- Crate `keyring` (v3, MIT OR Apache-2.0) with its macOS native backend. Keychain service
  `com.amornnan.shikigami`, account `anthropic-api-key`.
- Module `src-tauri/src/secrets/` with a small `SecretStore` trait: `get(account) ->
  Result<Option<String>>`, `set(account, value)`, `delete(account)` (deleting a missing item is
  not an error). Two implementations:
  - `KeychainStore` (always compiled): the `keyring` crate.
  - `MemoryStore` (compiled only with `#[cfg(any(test, feature = "e2e"))]`): an in-process map.
- Store selection at startup, managed as `Box<dyn SecretStore + Send + Sync>`:
  - with the `e2e` feature and `SHIKIGAMI_SECRET_STORE=memory`: `MemoryStore`, so e2e never
    touches the real Keychain and never triggers a Keychain prompt;
  - otherwise `KeychainStore`. The release build has no way to pick another store.
- Errors from the Keychain become a generic `SecretError` with a safe message ("could not read the
  saved API key from the Keychain"). The key value never appears in an error, log line or
  `Debug` output (wrap it in a newtype whose `Debug` prints `[redacted]`).

### Setting

- Migration `V2__settings.sql`: `CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);`
  Follow the Database rules in `CLAUDE.md` (grouped run, foreign keys off during migration).
- Setting key `claude-code.use-api-key`, value `true` or `false`. Missing means `true` when a key
  is saved (the switch is on by default once a key exists). Saving a key sets it to `true`.

### Commands (Rust, `spawn_blocking`)

- `api_key_status(provider) -> ApiKeyStatus { saved: bool, lastFour: Option<String>, enabled: bool }`.
- `save_api_key(provider, key) -> ApiKeyStatus`.
- `remove_api_key(provider) -> ApiKeyStatus` (also deletes the setting).
- `set_api_key_enabled(provider, enabled) -> ApiKeyStatus`.
- Only `claude-code` is supported; any other provider returns error code `unsupported`.
- Errors cross IPC as `{ code, message }` like the agents module: `key-invalid`, `unsupported`,
  `keychain-failed`, `database-failed`, `unavailable`.
- Key shape check (after trimming): starts with `sk-ant-`, 20-300 characters, no whitespace or
  control characters inside. Anything else is `key-invalid` with the message "That does not look
  like an Anthropic API key (it should start with sk-ant-)." The key is not validated online.
- `lastFour` is the last 4 characters of the saved key. Nothing else of the key is ever returned.
- For #17: `pub fn api_key_for_turn(app, provider) -> Result<Option<SecretString>>` returns the
  key only when it is saved and enabled. #17 passes it to the CLI as the `ANTHROPIC_API_KEY`
  environment variable. Write and test the function now; it has no caller yet (allow dead code only
  through a real `pub` export, not an attribute).

### Provider status

- `ProviderStatus` gains `apiKey: Option<ApiKeyStatus>`: present for `claude-code`, absent for
  `codex`. `list_providers` reads it (no CLI call).
- The UI shows Claude Code as "Ready (API key)" when the CLI is installed and a key is saved and
  enabled, whatever the CLI's own sign-in says. The sign-in line still shows below for
  information.

### UI (`src/features/providers/`)

- Under the Claude Code row, an "API key" block:
  - No key: an "Add API key" button that opens an inline form: a password input (no autofill,
    `autocomplete="off"`, spellcheck off), Save and Cancel. Errors under the input.
  - Key saved: "API key saved (…abcd)", a "Use API key" switch, and a Remove button with a
    confirmation ("Remove the saved API key? Claude Code agents will use the CLI sign-in.").
- After save, remove or toggle, invalidate the providers query.
- The input is cleared from React state right after a successful save.
- Use `@radix-ui/react-switch` (MIT, exact pin) for the switch, in `src/components/ui/switch.tsx`.

## Tests

- Rust (with `MemoryStore`):
  - shape validation: valid key, missing prefix, too short, too long, inner space, newline.
  - save then status gives saved, the right `lastFour`, enabled true; the store holds the trimmed
    key; the database holds no key text (scan every table's text columns for the key).
  - toggle off then status gives enabled false; `api_key_for_turn` gives None; toggle on gives the
    key.
  - remove deletes the item and the setting; removing twice is fine.
  - `codex` gives `unsupported`.
  - `Debug` of the secret newtype prints `[redacted]`.
  - migration V2 applies on top of a V1 database that has agents, and the agents survive.
- E2E (`SHIKIGAMI_SECRET_STORE=memory`, fake `claude`):
  - add a key: the row shows "API key saved (…" with the last 4 and "Ready (API key)";
  - a bad key shows the error;
  - toggle off: the status falls back to the CLI sign-in line;
  - remove: the block returns to "Add API key".
  - screenshots of each state.

## Manual check

With the real Keychain on this Mac: save a fake-shaped key (`sk-ant-test-0000000000000000`), confirm
`security find-generic-password -s com.amornnan.shikigami -a anthropic-api-key` finds the item
(without printing the secret: do not pass `-w`), confirm `sqlite3` finds no `sk-ant` text in the
temp database, remove it in the app, and confirm the item is gone. Never use or print the user's
real key.

## Out of scope

Codex keys, per-agent keys, online key validation, keeping the key away from agent commands (#29).

## Done

`bun run check` and `bun run e2e` pass; the manual check passes.
