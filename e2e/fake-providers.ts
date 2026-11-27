import { chmodSync, copyFileSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");

export const fakeProviderDir = join(root, "e2e", "artifacts", "fake-providers");
export const fakeClaudeVersionFile = join(fakeProviderDir, "claude-version");
export const fakeClaudeInvocations = join(fakeProviderDir, "invocations.log");
const fixtureDir = join(root, "src-tauri", "tests", "fixtures");
const fixtureNames = ["claude-stream-basic.jsonl", "claude-stream-denied.jsonl"];

export function fakeClaudeSessionId(): string {
  const [init = ""] = readFileSync(join(fixtureDir, fixtureNames[0] ?? ""), "utf8").split("\n");
  return JSON.parse(init).session_id;
}

export function setFakeClaudeVersion(version: string) {
  writeFileSync(fakeClaudeVersionFile, `${version}\n`);
}

export function prepareFakeProviders() {
  mkdirSync(fakeProviderDir, { recursive: true });
  setFakeClaudeVersion("9.9.9 (Fake Claude Code)");
  writeFileSync(fakeClaudeInvocations, "");
  for (const name of fixtureNames) {
    copyFileSync(join(fixtureDir, name), join(fakeProviderDir, name));
  }
  const fakeClaude = join(fakeProviderDir, "claude");
  writeFileSync(
    fakeClaude,
    `#!/bin/sh
PATH="$PATH:/usr/bin:/bin"
dir="\${0%/*}"
case "$1" in
  --version) read -r version < "$dir/claude-version"; echo "$version" ;;
  auth) echo '{"loggedIn": true}' ;;
  -p)
    echo "$*" >> "$dir/invocations.log"
    case " $* " in
      *" --setting-sources project --strict-mcp-config --permission-mode manual --permission-prompts none "*) ;;
      *) echo "Claude Code was not started isolated." >&2; exit 2 ;;
    esac
    case "$*" in
      *bypassPermissions*|*acceptEdits*|*dontAsk*|*dangerously*|*allowedTools*|*" auto"*)
        echo "Claude Code was started with a widened permission." >&2; exit 2 ;;
    esac
    message=$(cat)
    case "$message" in
      *files*) fixture="$dir/claude-stream-denied.jsonl" ;;
      *) fixture="$dir/claude-stream-basic.jsonl" ;;
    esac
    while IFS= read -r line; do
      printf '%s\\n' "$line"
      sleep 0.03
    done < "$fixture"
    ;;
esac
`,
  );
  chmodSync(fakeClaude, 0o755);
}
