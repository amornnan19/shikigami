# 0001. Libraries and tooling

- Status: accepted
- Date: 2026-11-27
- Issue: #1

## Context

Shikigami is a macOS desktop app. The Rust side runs agent CLIs as child processes, enforces the
sandbox, schedules routines and owns the SQLite database. The UI shows agents, chats and approvals.
The project should be fast to build on, small to ship, and every dependency must allow use in a
project licensed PolyForm Noncommercial 1.0.0.

The desktop shell (Tauri 2), the UI framework (React with TypeScript, Vite) and the package manager
(Bun) were chosen earlier.

## Decisions

### UI components and styling: shadcn/ui on Radix, with Tailwind CSS

- Options: shadcn/ui + Tailwind; Mantine; hand-written components with CSS modules.
- Chosen: shadcn/ui + Tailwind.
- Why: the components are copied into the repository, so we can change any of them. Radix gives
  accessible primitives (dialogs, menus, focus handling) that an approval-heavy UI needs. Tailwind
  keeps styles next to markup.

### UI state: TanStack Query for backend data, Zustand for screen state

- Options: TanStack Query + Zustand; Zustand only; Redux Toolkit.
- Chosen: TanStack Query + Zustand.
- Why: data from the Rust side (agents, messages, routines) gets caching, refetch and
  invalidation from TanStack Query. Small, local screen state (open panels, drafts) lives in
  Zustand. Both are small and have little boilerplate.

### SQLite access: rusqlite, with refinery for migrations

- Options: rusqlite + refinery; sqlx; the Tauri SQL plugin.
- Chosen: rusqlite + refinery.
- Why: a single-user desktop app needs direct control of SQLite (WAL, transactions, pragmas) more
  than an async pool. Migrations are plain SQL files run in order. The Tauri SQL plugin was rejected
  because it puts SQL and business rules in the UI, outside the Rust side that enforces the sandbox.

### Async runtime and processes: Tokio

- Chosen: Tokio (already used by Tauri), with Tokio's process support for agent CLIs.
- Serialization: serde and serde_json.

### TypeScript lint and format: Biome

- Options: Biome; ESLint + Prettier.
- Chosen: Biome. One fast tool for lint and format, one config file.

### Rust lint and format: rustfmt and clippy

- Standard tools, no alternative considered.

### Tests

- Rust: `cargo test`.
- UI: Vitest.
- End to end: WebdriverIO driving the real Tauri app.
  - The official `tauri-driver` supports only Windows and Linux; Apple ships no WebDriver for
    WKWebView.
  - On macOS we use WebdriverIO's Tauri service with its embedded provider, which runs a WebDriver
    server inside a debug build of the app through a plugin. The plugin is included in test builds
    only, never in the shipped app.
  - `@wdio/tauri-service` 1.5.1 supports WebdriverIO 9 only (peer `^9.0.0`). WebdriverIO 10 removed
    `browser.executeAsync`, which the service calls at start, so every worker failed in `before()`
    and retried (#25). We pin `@wdio/*` to 9.33.1. The service also looks for a companion plugin
    that this app does not include; the e2e config pins the session to its window so that lookup is
    skipped.
  - Its license is MIT, checked when it was added (#8). It works on macOS. If it stops working, the
    fallback is to test the UI in a browser with mocked backend calls, plus Rust integration tests.

### Secret storage: keyring, with the macOS Keychain

- Options: `keyring` 3 with the Keychain; a key file in the data folder; a key column in SQLite.
- Chosen: `keyring` 3.6.3 (`apple-native`), service `com.amornnan.shikigami`.
- Why: the API key must never be in the database or in a file, and the OS Keychain keeps it
  encrypted. The crate is MIT or Apache-2.0. Version 4 exists, but it is a different API; the
  plan for #28 names version 3.
- Test builds can use an in-memory store (the `e2e` feature; `SHIKIGAMI_SECRET_STORE=keychain` selects the real Keychain there) so
  tests never touch the real Keychain. The store is compiled out of every other build.

### Chat rendering: react-markdown with remark-gfm

- Options: `react-markdown` with `remark-gfm`; `marked` with a sanitizer and raw HTML; hand-written parsing.
- Chosen: `react-markdown` 10.1.0 and `remark-gfm` 4.0.1 (exact pins).
- Why: it turns Markdown into React elements and never inserts raw HTML, so agent output cannot add
  markup or scripts to the page. Links and code blocks are replaced by our own components (links open
  in the default browser; code blocks have a Copy button). `remark-gfm` adds tables and task lists.

### Moving a deleted agent's folder to the Trash: trash

- Options: `trash` 5.4.0; `osascript` with Finder; a hand-written call to `NSFileManager`.
- Chosen: `trash` 5.4.0 (exact pin, default features off), with its `NsFileManager` method.
- Why: the crate calls `trashItemAtURL` and needs no extra permission and no child process. The
  Finder method is the crate's default, but it runs `osascript` and can ask the user to allow
  automation. The Rust side checks that the folder is a real directory inside the workspaces folder
  before it calls the crate. Test builds (the `e2e` feature) replace the Trash with a plain move into
  the folder named by `SHIKIGAMI_E2E_TRASH_DIR`, and refuse to run without it, so tests never fill
  the user's Trash.

### Choosing a group folder: tauri-plugin-dialog

- Options: `tauri-plugin-dialog` 2.8.4; a hand-written `NSOpenPanel` call; `osascript` with Finder.
- Chosen: `tauri-plugin-dialog` 2.8.4 (exact pin), used from Rust only.
- Why: it opens the native macOS folder picker (through `rfd`) and returns the path to the Rust
  command, so the UI never gets a file-system permission and the Rust side checks the folder before
  it saves it. The plugin has no capability entry, so the UI cannot call its own commands. Test
  builds (the `e2e` feature) do not open the picker: `SHIKIGAMI_E2E_PICK_FOLDER` names the folder
  the command returns. Version 3 is an alpha and is not used.

## Licenses

| Dependency | License |
| --- | --- |
| Tauri 2 | MIT or Apache-2.0 |
| React, Vite | MIT |
| shadcn/ui, Radix UI | MIT |
| Tailwind CSS | MIT |
| TanStack Query | MIT |
| Zustand | MIT |
| rusqlite | MIT |
| refinery | MIT |
| Tokio | MIT |
| serde, serde_json | MIT or Apache-2.0 |
| uuid | MIT or Apache-2.0 |
| thiserror | MIT or Apache-2.0 |
| tempfile (tests only) | MIT or Apache-2.0 |
| Biome | MIT or Apache-2.0 |
| Vitest | MIT |
| WebdriverIO | MIT |
| `tauri-plugin-wdio-webdriver` (Tauri embedded WebDriver plugin, test builds only) | MIT |
| `@wdio/tauri-service` (WebdriverIO Tauri service) | MIT |
| `@tanstack/react-query` | MIT |
| `@radix-ui/react-slot` (used by the shadcn/ui button) | MIT |
| `@tauri-apps/plugin-opener` and `tauri-plugin-opener` (open the install page in the default browser) | MIT or Apache-2.0 |
| `tempfile` (Rust tests only) | MIT or Apache-2.0 |
| `@radix-ui/react-dialog`, `@radix-ui/react-select`, `@radix-ui/react-label`, `@radix-ui/react-radio-group` (shadcn/ui dialog, select, label and colour picker for the new agent form) | MIT |
| `keyring` 3.6.3 with its macOS native backend (saves the API key in the macOS Keychain) | MIT or Apache-2.0 |
| `security-framework` 3.7.0 (checks that the Keychain item exists without reading the secret) | MIT or Apache-2.0 |
| `zeroize` 1.9.2 (wipes the key from memory on drop) | Apache-2.0 or MIT |
| `react-markdown` 10.1.0 and `remark-gfm` 4.0.1 (render agent replies as Markdown) | MIT |
| `trash` 5.4.0 (moves a deleted agent's folder to the macOS Trash) | MIT |
| `tauri-plugin-dialog` 2.8.4 (native folder picker for a group chat's folder) | MIT or Apache-2.0 |
| `rfd` 0.17.2 and `tauri-plugin-fs` 2.7.1 (pulled in by `tauri-plugin-dialog`) | Apache-2.0 or MIT (`rfd`), MIT (`tauri-plugin-fs`) |
| `uuid` feature `v4` (random session ids for Claude Code; same crate as before) | MIT or Apache-2.0 |

All listed licenses are permissive and compatible with PolyForm Noncommercial 1.0.0. Their notices
go into the shipped app's third-party notices (M5).

## Consequences

- New dependencies need a license check before they are added.
- Business rules and the sandbox stay in Rust. The UI calls Rust commands and shows results.
