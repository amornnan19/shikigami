# Plan: issue #31, bump the version automatically on merge to main

## Goal

When work lands on `main`, the version in every manifest moves according to the Conventional
Commit types since the last release tag, through a release pull request that the workflow opens and
merges itself, then tags. Nobody edits version numbers by hand.

## Decisions

- Baseline: tag `v0.1.0` already points at `f5a7f56` (pushed). This PR's own commit type is `ci:`,
  so merging it does not bump.
- Script `scripts/bump-version.ts` (Bun, TypeScript). Pure functions exported for tests:
  - `parseCommit(subject, body) -> { type, breaking }`: Conventional Commit subject
    `type(scope)!: text`; `breaking` when `!` is present or the body has a line starting
    `BREAKING CHANGE:` or `BREAKING-CHANGE:`. A subject that does not match gives type `other`.
  - `decideBump(commits, current) -> "major" | "minor" | "patch" | null`: breaking gives major,
    but minor while `current` is below 1.0.0; else any `feat` gives minor; else any `fix` or
    `perf` gives patch; else null. Commits whose subject starts with `chore(release):` are ignored.
  - `nextVersion(current, bump) -> string`: semver `X.Y.Z` only; minor resets patch, major resets
    minor and patch.
- Script IO (in `main`, not exported):
  - newest tag: `git describe --tags --abbrev=0 --match "v[0-9]*"`; if none, all commits.
  - commits: `git log <tag>..HEAD --format=%s%x1f%b%x1e` (unit and record separators).
  - current version from `package.json`; fail if the three manifests disagree.
  - `--dry-run` (default when no flag): print the tag, the commits used with their parsed type, the
    bump and the next version; write nothing.
  - `--write`: set the version in `package.json` (`"version"`), `src-tauri/tauri.conf.json`
    (`"version"`), `src-tauri/Cargo.toml` (the `version =` line inside `[package]` only), and the
    `[[package]]` entry with `name = "shikigami"` in `src-tauri/Cargo.lock`. Edit text in place so
    formatting is kept (no JSON re-serialisation of whole files). Then print `version=<X.Y.Z>` and
    `bump=<kind>` lines, and append them to `$GITHUB_OUTPUT` when that variable is set. When there is
    no bump, print `bump=none` and write nothing.
- Workflow `.github/workflows/version.yml`:
  - `on: push: branches: [main]`.
  - Job `if: "!startsWith(github.event.head_commit.message, 'chore(release):')"`.
  - `permissions: contents: write, pull-requests: write`.
  - Steps: checkout with `fetch-depth: 0` and tags; `oven-sh/setup-bun@v2` with Bun 1.4.5;
    `bun install --frozen-lockfile`; `bun scripts/bump-version.ts --write` with `id: bump`.
  - If `steps.bump.outputs.bump != 'none'`:
    - configure git as `github-actions[bot]` (`41898282+github-actions[bot]@users.noreply.github.com`);
    - branch `release/v<version>`, commit `chore(release): v<version>`, push;
    - `gh pr create --base main --head release/v<version> --title "chore(release): v<version>" --body "Automated version bump from the commits since the previous tag."`;
    - `gh pr merge release/v<version> --rebase --delete-branch`, retried up to 10 times, 6 s apart,
      while GitHub reports the PR as not yet mergeable (bounded loop with a counter);
    - `git fetch origin main`, find the merged commit as `origin/main` (the release commit is now
      its tip), `git tag v<version> origin/main`, `git push origin v<version>`.
  - `GH_TOKEN: ${{ github.token }}` for `gh`. Concurrency group `version` with
    `cancel-in-progress: false` so two merges queue instead of racing.
- Repository setting already enabled: Actions may create pull requests.
- Vitest must collect `scripts/**/*.test.ts`. If the current Vitest config only looks in `src/`, add
  `scripts` to its include list.

## Tests (Vitest, `scripts/bump-version.test.ts`)

- `parseCommit`: `feat: x`, `fix(ui): x`, `feat!: x`, `refactor: x` with a `BREAKING CHANGE:` footer,
  `Merge branch ...` gives `other`.
- `decideBump`: only docs/chore/test/ci/refactor/style gives null; one fix gives patch; fix plus feat
  gives minor; breaking at 0.4.2 gives minor; breaking at 1.2.3 gives major; `chore(release): v0.2.0`
  commits are ignored; empty list gives null.
- `nextVersion`: 0.1.0 + patch = 0.1.1, + minor = 0.2.0, 1.2.3 + major = 2.0.0; a non-semver input
  throws.

## Manual check

- `bun scripts/bump-version.ts --dry-run` on this branch prints tag `v0.1.0` and the commits after it
  (the `ci:` commit of this branch), and `bump=none`.
- In a temp clone (`git clone` of the local repo into the scratchpad), create a `feat:` commit and run
  `--write`: the three manifests and Cargo.lock show 0.2.0 and nothing else changed
  (`git diff --stat`). Delete the temp clone.

## Out of scope

Building or publishing release artifacts, changelogs.

## Done

`bun run check` passes with the new tests; the manual check passes; after merge, the next `feat:`
merge produces a `chore(release): v0.2.0` PR that merges itself and a `v0.2.0` tag (checked when #17
lands).
