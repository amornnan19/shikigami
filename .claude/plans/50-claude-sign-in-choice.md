# Plan: issue #50, choose Claude subscription or API key, per app and per agent

## Goal

The user picks how Claude agents authenticate: their Claude subscription (the CLI's own sign-in)
or the saved Anthropic API key. One app-wide default, which any agent can override. This replaces
the "Use API key" switch.

## Decisions

### Data (migration V3, follow the Database rules in `CLAUDE.md`)

- `ALTER TABLE agents ADD COLUMN auth_mode TEXT CHECK (auth_mode IS NULL OR auth_mode IN
  ('subscription', 'api-key'))`. NULL means "use the app default".
- App default in `settings` under `claude.auth-mode` (`subscription` or `api-key`).
- In the same migration, convert the old switch: if `claude-code.use-api-key` is `true`, insert
  `claude.auth-mode = api-key`; if `false`, insert `subscription`; then delete the old key.
- When `claude.auth-mode` is absent, the effective app default is `api-key` if a key item exists in
  the Keychain (attributes-only check, never a decrypt), otherwise `subscription`.

### Rust

- Module `src-tauri/src/auth/` (`mod.rs`): `AuthMode { Subscription, ApiKey }`,
  `effective_mode(agent_override, app_setting, key_exists) -> AuthMode`.
- Commands:
  - `get_claude_auth() -> { appMode, appModeExplicit: bool, subscription: { installed, version,
    signedIn: "yes" | "no" | "unknown", signInHint }, apiKey: { saved, lastFour, readFailed } }`
    (no decrypt; reuses the providers probe and the api_key status).
  - `set_claude_auth_mode(mode)`.
  - `set_agent_auth_mode(agentId, mode | null) -> Agent`.
  - `create_agent` input gains optional `authMode` (absent or null = app default).
- `Agent` gains `authMode: "subscription" | "api-key" | null` and `effectiveAuthMode`.
- Remove `set_api_key_enabled` and the `enabled` field of the API key status. Save and Remove of
  the key stay.
- Turn start (`turns`): resolve the effective mode for the agent, then:
  - `subscription`: remove `ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN`, `ANTHROPIC_BASE_URL` from the
    child environment; never touch the Keychain.
  - `api-key`: read the key (decrypt only here). No item: fail the turn with
    "No API key is saved. Add one or switch this agent to your Claude subscription." and start no
    process (a failure before spawn: the queue does not advance). Read failure: as today
    ("Could not read the saved API key…").
- `api_key_for_turn` becomes `key_for_turn(mode)`: returns `None` without any Keychain access in
  subscription mode.

### UI

- The Providers card becomes "Claude":
  - a line with the CLI status (Ready with version / Not installed with Install);
  - "Sign in with" radio group:
    - "Claude subscription": below it "Signed in" / "Not signed in — run `claude auth login` in a
      terminal" (copy button) / "Sign-in status unknown";
    - "API key": below it "API key saved (…abcd)" with Remove, or "Add API key" with the existing
      form.
  - Choosing an option calls `set_claude_auth_mode`. Choosing "API key" with no key opens the add
  form.
- Create dialog: "Sign-in" select: "Use app default (Subscription)" / "(API key)", "Claude
  subscription", "API key". Default: app default.
- Chat header: a small badge after the model, "Subscription" or "API key".

## Tests

- Rust: `effective_mode` table (override wins both ways; absent setting falls back on key
  existence); the V3 migration converts `true`/`false`/absent and keeps agents; a subscription turn
  has no Anthropic variables and never calls the store (store double panics on any call); an
  api-key turn with no item fails before spawn with the message and does not advance the queue;
  override via `set_agent_auth_mode`; `get_claude_auth` never decrypts.
- E2E: choose API key in the Claude card, add a key, create an agent with "Use app default", see the
  "API key" badge; switch the app to subscription and see the badge change; create an agent with
  an explicit "API key" override and see it keep "API key" after the app switch. Screenshots.

## Manual check

One real turn with `claude-haiku-5-5` in subscription mode (no Keychain access, so no prompt);
confirm in the captured init line `apiKeySource` is `none`.

## Out of scope

Agent settings UI for the override (#20 adds it), several keys or accounts.

## Done

`bun run check`; `bun run e2e` 3/3; the manual check.
