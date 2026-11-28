import {
  chmodSync,
  copyFileSync,
  mkdirSync,
  readdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");

export const fakeProviderDir = join(root, "e2e", "artifacts", "fake-providers");
export const fakeClaudeVersionFile = join(fakeProviderDir, "claude-version");
export const fakeClaudeInvocations = join(fakeProviderDir, "invocations.log");
export const fakeClaudeCwds = join(fakeProviderDir, "cwd.log");
export const fakeClaudeStdins = join(fakeProviderDir, "stdin.log");
export const fakeClaudePrompts = join(fakeProviderDir, "prompt.log");
export const fakeClaudeSettings = join(fakeProviderDir, "settings.log");
export const fakeClaudeToolMarker = join(fakeProviderDir, "tool-ran");
export const fakeClaudeRecordSeparator = "\n----\n";
const fixtureDir = join(root, "src-tauri", "tests", "fixtures");
const fixtureNames = ["claude-stream-basic.jsonl", "claude-stream-denied.jsonl"];

export function fakeClaudeSessionId(): string {
  const [init = ""] = readFileSync(join(fixtureDir, fixtureNames[0] ?? ""), "utf8").split("\n");
  return JSON.parse(init).session_id;
}

export function scriptFakeReply(call: number, text: string) {
  const reply = JSON.stringify(text);
  const script = readFileSync(join(fakeProviderDir, fixtureNames[0] ?? ""), "utf8")
    .replaceAll('"text":"po"', `"text":${reply}`)
    .replaceAll('"text":"ng"', '"text":""')
    .replaceAll('"text":"pong"', `"text":${reply}`)
    .replaceAll('"result":"pong"', `"result":${reply}`);
  writeFileSync(join(fakeProviderDir, `reply.${call}.jsonl`), script);
}

export function scriptFakeStream(call: number, lines: (object | string)[]) {
  const body = lines.map((line) => (typeof line === "string" ? line : JSON.stringify(line)));
  writeFileSync(join(fakeProviderDir, `reply.${call}.jsonl`), `${body.join("\n")}\n`);
}

export function setFakeClaudeVersion(version: string) {
  writeFileSync(fakeClaudeVersionFile, `${version}\n`);
}

export function prepareFakeProviders() {
  mkdirSync(fakeProviderDir, { recursive: true });
  setFakeClaudeVersion("9.9.9 (Fake Claude Code)");
  for (const log of [
    fakeClaudeInvocations,
    fakeClaudeCwds,
    fakeClaudeStdins,
    fakeClaudePrompts,
    fakeClaudeSettings,
  ]) {
    writeFileSync(log, "");
  }
  for (const name of fixtureNames) {
    copyFileSync(join(fixtureDir, name), join(fakeProviderDir, name));
  }
  rmSync(fakeClaudeToolMarker, { force: true });
  for (const name of readdirSync(fakeProviderDir)) {
    if (name.startsWith("reply.")) rmSync(join(fakeProviderDir, name));
  }
  const fakeClaude = join(fakeProviderDir, "claude");
  writeFileSync(
    fakeClaude,
    `#!/bin/sh
PATH="$PATH:/usr/bin:/bin"
dir="\${0%/*}"
case "$1" in
  --help) echo "  --system-prompt-snapshot <on|off>" ;;
  --version) read -r version < "$dir/claude-version"; echo "$version" ;;
  auth) echo '{"loggedIn": true}' ;;
  -p)
    safe=""
    prompt=""
    take=0
    for arg in "$@"; do
      if [ "$take" = 1 ]; then prompt=$(cat "$arg"); take=0; continue; fi
      if [ "$arg" = "--append-system-prompt-file" ]; then take=1; safe="$safe $arg"; continue; fi
      safe="$safe $arg"
    done
    echo "$safe" >> "$dir/invocations.log"
    case "$safe " in
      *" --setting-sources  --strict-mcp-config --permission-mode manual --permission-prompts none "*) ;;
      *) echo "Claude Code was not started isolated." >&2; exit 2 ;;
    esac
    case "$safe" in
      *bypassPermissions*|*acceptEdits*|*dontAsk*|*dangerously*|*allowedTools*|*" auto"*)
        echo "Claude Code was started with a widened permission." >&2; exit 2 ;;
    esac
    prev=""
    for arg in "$@"; do
      if [ "$prev" = "--settings" ]; then
        cat "$arg" >> "$dir/settings.log"
        printf '\\n----\\n' >> "$dir/settings.log"
      fi
      prev="$arg"
    done
    message=$(cat)
    pwd -P >> "$dir/cwd.log"
    printf '%s\\n----\\n' "$message" >> "$dir/stdin.log"
    if [ -n "$prompt" ]; then printf '%s\\n----\\n' "$prompt" >> "$dir/prompt.log"; fi
    call=$(wc -l < "$dir/invocations.log" | tr -d ' ')
    if [ -f "$dir/reply.$call.jsonl" ]; then
      while IFS= read -r line; do
        case "$line" in
          "#sleep "*) sleep "\${line#"#sleep "}" ;;
          *) printf '%s\\n' "$line"; sleep 0.03 ;;
        esac
      done < "$dir/reply.$call.jsonl"
      exit 0
    fi
    case "$message" in
      *IGNORESETTINGS*)
        printf '%s\\n' '{"type":"system","subtype":"init","session_id":"ignored","tools":["Read","Bash","NotebookEdit"]}'
        sleep 2
        touch "$dir/tool-ran"
        exit 0 ;;
      *NOSANDBOX*)
        echo "Error: sandbox required but unavailable: seatbelt is missing" >&2
        echo "  Refusing to start without a working sandbox (see sandbox.failIfUnavailable)." >&2
        exit 1 ;;
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
      *SIGNIN*)
        head -n 4 "$dir/claude-stream-basic.jsonl"
        sleep 3
        printf '%s\\n' '{"type":"result","subtype":"success","is_error":true,"api_error_status":401,"result":"Invalid API key · Please run /login"}'
        exit 1 ;;
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
