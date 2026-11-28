# Plan: issue #92

## Goal
Close the last key path and harden the settings canary found in review of #89.

## Behaviour
- The saved API key never sits in the agent CLI's own environment, so `ps -E` cannot show it: deliver it through Claude Code's `apiKeyHelper` set in the turn's `--settings`, pointing at a small helper that prints the key from a 0600 file in Shikigami's data folder (protected from the agent), created right before the turn and removed when it ends (and swept at startup).
- The canary judges only the first init event; any model or tool event before it fails closed; later inits fail only if the canary appears. Buffered lines after the kill are not stored or shown. The init list must also contain Read, Edit and Write, or the turn fails closed.
- A group member's own unusable workspace CLAUDE.md shows a notice in group turns too.

## Acceptance checks
- [ ] In API-key mode (fake CLI), the child environment has no ANTHROPIC_API_KEY, the settings carry apiKeyHelper, and the key file is 0600, inside the data folder, and gone after the turn.
- [ ] A probe with a stand-in key and a helper shows the CLI authenticates through the helper (or the documented behaviour is recorded if it cannot be probed without the Keychain).
- [ ] Canary: a stream starting with an assistant event, a later init without tools, an init missing Read/Edit/Write each behave as described; buffered lines after a kill are not stored.

Depends on #89.


## Decisions
- The helper is a 0700 shell script `key-helper-<uuid>.sh` in `<data>/tmp` whose only content is `#!/bin/sh` and `exec /bin/cat '<data>/tmp/key-<uuid>'`; the key file is 0600 with the key and no newline. Both are created with create_new, removed when the turn ends (guard drop), swept at startup (`key-*` and `key-helper-*`). Read Claude Code's apiKeyHelper docs for refresh and caching semantics and confirm it works under `--setting-sources ""` via `--settings` (the CLI help mentions apiKeyHelper via --settings).
- `ANTHROPIC_API_KEY` is no longer set on the child in API-key mode; keep the sandbox `credentials.envVars` deny and the SCRUB variable as extra layers.
- Probe without the Keychain: a raw run whose helper prints an invalid stand-in key must fail with an authentication error (proving the helper is used); the subscription sign-in must not be used in that run. Record the CLI's message.

## Done
`bun run check`; 10x `cargo test --lib`; `bun run e2e` 3/3; probe recorded.
