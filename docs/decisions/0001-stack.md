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

All listed licenses are permissive and compatible with PolyForm Noncommercial 1.0.0. Their notices
go into the shipped app's third-party notices (M5).

## Consequences

- New dependencies need a license check before they are added.
- Business rules and the sandbox stay in Rust. The UI calls Rust commands and shows results.
