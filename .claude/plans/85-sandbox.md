# Plan: issue #85, run every agent turn in a sandbox and fail closed

## Goal

Every turn runs with Claude Code's own sandbox switched on and configured by Shikigami, so shell
commands write only inside the agent's folder and the temporary folder, cannot read protected
locations and have no network, and every tool is refused on protected locations and on writes
outside the folder. If the sandbox cannot start, the turn fails and nothing runs.

## Decisions

### How (Claude Code settings passed by Shikigami)

- Shikigami builds one settings JSON per turn and passes it with `--settings <path>` (a 0600 temp
  file in `<data>/tmp`, created and removed like the instructions file, swept at startup by the same
  sweep: extend the pattern to `settings-*.json`). `--settings` applies even with
  `--setting-sources ""` (CLI help); confirm with the probe below.
- New module `src-tauri/src/sandbox/mod.rs` (pure, unit-tested): `turn_settings(folder: &Path,
  temp: &Path, home: &Path, data_dir: &Path, other_workspaces: &[PathBuf]) -> serde_json::Value`.
  Content:
  - `sandbox`: `enabled: true`, `failIfUnavailable: true`, `allowUnsandboxedCommands: false`,
    `autoAllowBashIfSandboxed: false`, `network: { allowedDomains: [], strictAllowlist: true }`,
    `filesystem: { allowWrite: [folder, temp], denyRead: <protected> }`.
  - `permissions`: `allow`: `Read(<folder>/**)`, `Edit(<folder>/**)` (Edit covers every
    file-writing tool), `Glob`/`Grep` limited to the folder if the rule syntax supports paths;
    `deny`: `Read(<p>/**)` and `Edit(<p>/**)` for every protected path, and `Edit(/**)` is NOT used
    (writes outside the folder are refused because they are not allowed and prompts are off;
    confirm in the probe and add an explicit deny if they are not).
  - Use absolute paths in the rule syntax the CLI documents (read the permission-rule docs; a path
    rule may need `//` for absolute paths). Unit tests pin the exact strings.
- Protected locations (all under the real home unless absolute): `.ssh`, `.gnupg`, `.aws`,
  `.azure`, `.kube`, `.docker`, `.config/gcloud`, `.config/gh`, `.npmrc`, `.pypirc`, `.netrc`,
  `.git-credentials`, `.claude`, `.claude.json`, `Library/Keychains`, `Library/Mail`,
  `Library/Messages`, `Library/Safari`, `Library/Application Support/Google/Chrome`,
  `Library/Application Support/Firefox`, `Library/Application Support/BraveSoftware`,
  `Library/Application Support/Microsoft Edge`, `Library/Application Support/Arc`,
  `Library/Cookies`, `.zsh_history`, `.bash_history`, `.zsh_sessions`, `.zshrc`, `.zshenv`,
  `.zprofile`, `.zlogin`, `.bashrc`, `.bash_profile`, `.profile`; plus Shikigami's data folder and
  every other agent's workspace (recorded paths from the agents table, excluding the folder of this
  turn). A protected path equal to or containing the turn's folder is left out of the list (it cannot
  be, by the folder rules, but never deny the folder itself).
- The same settings apply to one-to-one and group turns; the folder is the turn's cwd.
- Fail closed: when the CLI exits because the sandbox is unavailable (identify its stderr or result
  text in the probe), the turn ends failed with "The sandbox could not start, so this agent did not
  run." and the run ends `AgentFailed` (it holds the agent like other agent-level failures).

### Probe (real CLI, before building on assumptions)

At most 4 raw `claude -p` runs, model `claude-haiku-5-5`, subscription mode, in a temp folder, with
the isolation flags, `--settings <generated file>` and, for the probe only, an extra allow rule
`Bash` so commands reach the sandbox: ask it to run, in one turn each or combined:
`cat ~/.ssh/known_hosts` (or `ls ~/.ssh`), `touch ~/shikigami-probe-outside` (must fail; if it
succeeds, delete that file and report), `curl -sS https://example.com` (must fail), and the Read tool
on `~/.zshrc` (must be denied). Also a run with `--setting-sources ""` to confirm `--settings` still
applies. Record outputs in the report. Never print secrets: use `ls` not `cat` on `~/.ssh`.

### Tests

- Unit: `turn_settings` for a workspace folder and a group folder: exact JSON; other agents'
  workspaces present; the turn's own folder never denied; data folder denied.
- Runner: argv contains `--settings` with a file whose content equals `turn_settings`; the file is
  0600 and removed after the turn; a fake CLI that prints the sandbox-unavailable text ends the turn
  failed with the message and holds the agent.
- Startup sweep removes leftover `settings-*.json`.
- E2E: fake CLI records the settings file content; one assertion per spec check that fakes can show
  (the settings carry the folder, protected paths, no network); the sandbox-unavailable message in
  the UI with a screenshot.

## Out of scope

Approval cards (#86), the API key (#29), the user's own setup (#38).

## Done

`bun run check`; 10x `cargo test --lib`; `bun run e2e` 3/3; the probe results in the report.
