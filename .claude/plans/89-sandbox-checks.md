# Plan: issues #89 and #29

## Goal
Close the gaps left after #85 so the sandbox stays fail-closed when the CLI changes.

## Behaviour
- Verify the settings took effect before any tool runs: the settings carry a canary (a bare-name deny for a tool Shikigami never needs); when the CLI's init event lists that tool, or lists no tools, the turn is stopped at once and fails closed with the sandbox message, holding the agent. Probe on the real CLI that the canary disappears from the init tool list.
- A stderr settings-not-loaded marker fails the turn even if the turn otherwise succeeded.
- A Claude Code version that cannot be read (timeout, unexpected output) shows "Could not check the Claude Code version. Press Resume to try again." instead of "too old"; the version is found on any output line, not only the first.
- A one-to-one workspace CLAUDE.md that exists but cannot be used (too large, a link, unreadable, or with more than one hard link) shows a turn notice, like group turns; files with more than one hard link are refused for one-to-one and group instructions.
- Groups whose folder is on an external disk (under /Volumes): stay refused for new groups; an existing group in that place fails its turns with the protected-location reason (consistent with the pick rule).

## Acceptance checks
- [ ] A CLI that ignores the settings file (fake that lists the canary tool in init) is stopped before any tool runs and the turn fails with the sandbox message.
- [ ] The canary is absent from init tools on the real CLI (probe recorded).
- [ ] A version probe timeout shows the could-not-check message, not "too old".
- [ ] An unusable one-to-one CLAUDE.md shows a notice; a hard-linked one is refused.

Depends on #85. Spec: docs/specs/sandbox-and-approval.md


## Also #29: keep the API key away from agent commands

- In API-key mode the CLI still gets `ANTHROPIC_API_KEY` in its environment, but commands it runs
  must not see it. Use both layers: `sandbox.credentials.envVars` with `ANTHROPIC_API_KEY` set to
  `deny` in the generated settings (read the docs for the exact shape), and
  `CLAUDE_CODE_SUBPROCESS_ENV_SCRUB=1` in the child environment if the docs confirm that variable.
- Probe (real CLI, API-key mode is NOT allowed: the Keychain must not be read). Instead export a
  dummy `ANTHROPIC_API_KEY=sk-ant-dummy-probe` in a raw `claude -p` run that signs in by
  subscription... if the CLI then refuses to authenticate, use a fake key only with a fake CLI and
  rely on the documented behaviour; record which. In a raw run with the dummy variable set and a
  probe-only `Bash(printenv:*)` allow rule, `printenv ANTHROPIC_API_KEY` must print nothing.
- Test: the generated settings carry the deny entry; the child env carries the scrub variable in
  API-key mode; redaction still covers the key.

## Done

`bun run check`; 10x `cargo test --lib`; `bun run e2e` 3/3; probes recorded. Closes #89 and #29.
