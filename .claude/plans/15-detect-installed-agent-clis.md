# Plan: issue #15, detect installed agent CLIs

## Goal

The Rust side knows, for Claude Code and Codex, whether the CLI is installed, its version, and
whether the user is signed in, and the UI can show it with clear next steps.

## Decisions

- Module `src-tauri/src/providers/` with `mod.rs` (types, `detect_all`), `search.rs` (finding an
  executable), `claude_code.rs` and `codex.rs` (per-provider probes).
- Provider ids match the database: `claude-code`, `codex`. Executables: `claude`, `codex`.
- Search path: a GUI app does not inherit the shell PATH, so search, in order and without
  duplicates: the process PATH, then `~/.local/bin`, `/opt/homebrew/bin`, `/usr/local/bin`,
  `~/.bun/bin`, `~/.npm-global/bin`, `~/.volta/bin`. The first executable file found wins. Do not
  run a login shell to read PATH.
- `SHIKIGAMI_PROVIDER_PATH` (colon-separated) replaces the whole search list when set. Tests and e2e
  use it so they never depend on the developer's machine.
- Probes run the CLI with a 5-second timeout each, no shell, with the found absolute path, and
  never with arguments that start a session or change anything:
  - version: `<cli> --version`, first line trimmed.
  - Claude Code sign-in: `claude auth status`, which prints JSON with a boolean `loggedIn`.
  - Codex sign-in: `codex login status`; exit code 0 means signed in, non-zero means signed out.
  - A probe that times out, fails to start, or prints something unparseable gives `unknown` for that
    field, never an error for the whole detection.
- Result type, serialised to the UI with serde (camelCase):
  `ProviderStatus { id, name, installed: bool, path: Option<String>, version: Option<String>,
  signedIn: "yes" | "no" | "unknown", installHint, signInHint }`.
  - `name`: "Claude Code", "Codex".
  - `installHint`: the official install page URL for each CLI.
    Claude Code: `https://docs.anthropic.com/en/docs/claude-code/setup`.
    Codex: `https://github.com/openai/codex`.
  - `signInHint`: the command to run in a terminal: `claude auth login`, `codex login`.
- Tauri command `list_providers` (async, runs probes on a blocking thread, both providers in
  parallel) returns `Vec<ProviderStatus>`. No caching in M1; the UI calls it on load and on a
  Refresh button.
- UI: on the ok screen, below the placeholder, a "Providers" section lists each provider with a
  status line: "Ready" with version; "Not installed" with an "Install" link (opens the default
  browser via the Tauri opener plugin) ; "Not signed in" with the sign-in command in a copyable code
  block; "Sign-in status unknown" with the version. A Refresh button re-runs detection. Use
  shadcn/ui components (add `button` and `card` with the shadcn CLI or by hand) and Tailwind.
  Data fetching through TanStack Query (add it now, exact pin), per the stack decision.

## Tests

- Rust unit tests with fake executables (small shell scripts written to a temp dir, made
  executable, pointed at by `SHIKIGAMI_PROVIDER_PATH` or by passing the search list directly):
  - missing executable gives `installed: false`;
  - a fake `claude` printing a version and `{"loggedIn": true}` gives ready, signed in yes;
  - `{"loggedIn": false}` gives signed in no;
  - a fake `codex` whose `login status` exits 1 gives signed in no;
  - a probe that sleeps longer than the timeout gives unknown within the timeout (make the timeout
    injectable so the test uses ~200 ms);
  - garbage output gives unknown.
- E2E: start the app with `SHIKIGAMI_PROVIDER_PATH` pointing at a temp dir holding a fake `claude`
  (ready) and no `codex`; assert "Ready" for Claude Code and "Not installed" for Codex; screenshot.

## Out of scope

Installing or updating CLIs, API keys, using the result in the create form (that is #16).

## Done

`bun run check` and `bun run e2e` pass (e2e 3/3); the dev app on this Mac shows Claude Code ready
with its real version and Codex not installed.
