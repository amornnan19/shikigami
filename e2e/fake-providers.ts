import { chmodSync, copyFileSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");

export const fakeProviderDir = join(root, "e2e", "artifacts", "fake-providers");
export const fakeClaudeVersionFile = join(fakeProviderDir, "claude-version");
export const fakeClaudeInvocations = join(fakeProviderDir, "invocations.log");
export const fakeClaudeCwds = join(fakeProviderDir, "cwd.log");
export const fakeClaudeStdins = join(fakeProviderDir, "stdin.log");
export const fakeClaudePrompts = join(fakeProviderDir, "prompt.log");
export const fakeClaudeRecordSeparator = "\n----\n";
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
  for (const log of [fakeClaudeInvocations, fakeClaudeCwds, fakeClaudeStdins, fakeClaudePrompts]) {
    writeFileSync(log, "");
  }
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
    safe=""
    prompt=""
    take=0
    for arg in "$@"; do
      if [ "$take" = 1 ]; then prompt="$arg"; take=0; continue; fi
      if [ "$arg" = "--append-system-prompt" ]; then take=1; continue; fi
      safe="$safe $arg"
    done
    echo "$safe" >> "$dir/invocations.log"
    case "$safe " in
      *" --setting-sources project --strict-mcp-config --permission-mode manual --permission-prompts none "*) ;;
      *) echo "Claude Code was not started isolated." >&2; exit 2 ;;
    esac
    case "$safe" in
      *bypassPermissions*|*acceptEdits*|*dontAsk*|*dangerously*|*allowedTools*|*" auto"*)
        echo "Claude Code was started with a widened permission." >&2; exit 2 ;;
    esac
    message=$(cat)
    pwd -P >> "$dir/cwd.log"
    printf '%s\\n----\\n' "$message" >> "$dir/stdin.log"
    if [ -n "$prompt" ]; then printf '%s\\n----\\n' "$prompt" >> "$dir/prompt.log"; fi
    case "$message" in
      *STALL*)
        head -n 4 "$dir/claude-stream-basic.jsonl"
        sleep 60
        exit 0 ;;
      *SLOW*)
        head -n 4 "$dir/claude-stream-basic.jsonl"
        nums=$(seq -s ' ' 1 60)
        printf '{"type":"stream_event","api_message_id":"m1","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Counting slowly. %s"}}}\\n' "$nums"
        sleep 7
        tail -n +5 "$dir/claude-stream-basic.jsonl"
        exit 0 ;;
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
