import { execFileSync } from "node:child_process";
import { appendFileSync, readFileSync, writeFileSync } from "node:fs";
import path from "node:path";

export type CommitType = string;
export type Bump = "major" | "minor" | "patch";
export type Commit = { subject: string; body: string };
export type ParsedCommit = { type: CommitType; breaking: boolean };

const SUBJECT = /^(\w+)(\([^)]*\))?(!)?: .+/;
const BREAKING_FOOTER = /^BREAKING[ -]CHANGE:/m;
const SEMVER = /^(\d+)\.(\d+)\.(\d+)$/;
const RELEASE_PREFIX = "chore(release):";

export function parseCommit(subject: string, body: string): ParsedCommit {
  const match = SUBJECT.exec(subject);
  if (!match) {
    return { type: "other", breaking: false };
  }
  return {
    type: match[1].toLowerCase(),
    breaking: match[3] === "!" || BREAKING_FOOTER.test(body),
  };
}

function parseSemver(version: string): [number, number, number] {
  const match = SEMVER.exec(version);
  if (!match) {
    throw new Error(`Not a semver X.Y.Z version: ${version}`);
  }
  return [Number(match[1]), Number(match[2]), Number(match[3])];
}

export function decideBump(commits: Commit[], current: string): Bump | null {
  const [major] = parseSemver(current);
  const parsed = commits
    .filter((commit) => !commit.subject.startsWith(RELEASE_PREFIX))
    .map((commit) => parseCommit(commit.subject, commit.body));
  if (parsed.some((commit) => commit.breaking)) {
    return major < 1 ? "minor" : "major";
  }
  if (parsed.some((commit) => commit.type === "feat")) {
    return "minor";
  }
  if (parsed.some((commit) => commit.type === "fix" || commit.type === "perf")) {
    return "patch";
  }
  return null;
}

export function nextVersion(current: string, bump: Bump): string {
  const [major, minor, patch] = parseSemver(current);
  if (bump === "major") {
    return `${major + 1}.0.0`;
  }
  if (bump === "minor") {
    return `${major}.${minor + 1}.0`;
  }
  return `${major}.${minor}.${patch + 1}`;
}

const root = path.resolve(import.meta.dirname, "..");
const PACKAGE_JSON = path.join(root, "package.json");
const TAURI_CONF = path.join(root, "src-tauri/tauri.conf.json");
const CARGO_TOML = path.join(root, "src-tauri/Cargo.toml");
const CARGO_LOCK = path.join(root, "src-tauri/Cargo.lock");

function git(args: string[]): string {
  return execFileSync("git", args, { cwd: root, encoding: "utf8" });
}

function newestTag(): string | null {
  try {
    return git(["describe", "--tags", "--abbrev=0", "--match", "v[0-9]*"]).trim();
  } catch {
    return null;
  }
}

function commitsSince(tag: string | null): Commit[] {
  const range = tag ? [`${tag}..HEAD`] : ["HEAD"];
  const raw = git(["log", ...range, "--format=%s%x1f%b%x1e"]);
  return raw
    .split("\x1e")
    .map((record) => record.replace(/^\n/, ""))
    .filter((record) => record.trim() !== "")
    .map((record) => {
      const [subject, body = ""] = record.split("\x1f");
      return { subject, body };
    });
}

function jsonVersion(file: string): string {
  const match = /^ {2}"version": "([^"]*)"/m.exec(readFileSync(file, "utf8"));
  if (!match) {
    throw new Error(`No version in ${file}`);
  }
  return match[1];
}

function cargoTomlVersion(text: string): string {
  const match = /^\[package\]\n(?:(?!\[).*\n)*?version = "([^"]*)"/m.exec(text);
  if (!match) {
    throw new Error(`No package version in ${CARGO_TOML}`);
  }
  return match[1];
}

function currentVersion(): string {
  const versions = {
    [PACKAGE_JSON]: jsonVersion(PACKAGE_JSON),
    [TAURI_CONF]: jsonVersion(TAURI_CONF),
    [CARGO_TOML]: cargoTomlVersion(readFileSync(CARGO_TOML, "utf8")),
  };
  const distinct = new Set(Object.values(versions));
  if (distinct.size !== 1) {
    throw new Error(`Manifest versions disagree: ${JSON.stringify(versions)}`);
  }
  return [...distinct][0];
}

function replaceOnce(text: string, pattern: RegExp, replacement: string, file: string): string {
  if (!pattern.test(text)) {
    throw new Error(`Pattern not found in ${file}`);
  }
  return text.replace(pattern, replacement);
}

function writeVersion(version: string): void {
  const jsonPattern = /^( {2}"version": ")[^"]*(")/m;
  for (const file of [PACKAGE_JSON, TAURI_CONF]) {
    const text = readFileSync(file, "utf8");
    writeFileSync(file, replaceOnce(text, jsonPattern, `$1${version}$2`, file));
  }
  const toml = readFileSync(CARGO_TOML, "utf8");
  const tomlPattern = /^(\[package\]\n(?:(?!\[).*\n)*?version = ")[^"]*(")/m;
  writeFileSync(CARGO_TOML, replaceOnce(toml, tomlPattern, `$1${version}$2`, CARGO_TOML));
  const lock = readFileSync(CARGO_LOCK, "utf8");
  const lockPattern = /^(name = "shikigami"\nversion = ")[^"]*(")/m;
  writeFileSync(CARGO_LOCK, replaceOnce(lock, lockPattern, `$1${version}$2`, CARGO_LOCK));
}

function main(): void {
  const write = process.argv.includes("--write");
  const tag = newestTag();
  const commits = commitsSince(tag);
  const current = currentVersion();
  const bump = decideBump(commits, current);
  const next = bump ? nextVersion(current, bump) : null;

  if (!write) {
    console.log(`tag=${tag ?? "none"}`);
    console.log(`current=${current}`);
    for (const commit of commits) {
      const { type, breaking } = parseCommit(commit.subject, commit.body);
      const ignored = commit.subject.startsWith(RELEASE_PREFIX) ? " ignored" : "";
      console.log(`  [${type}${breaking ? "!" : ""}${ignored}] ${commit.subject}`);
    }
    console.log(`bump=${bump ?? "none"}`);
    console.log(`next=${next ?? current}`);
    return;
  }

  const lines = next && bump ? [`version=${next}`, `bump=${bump}`] : ["bump=none"];
  if (next) {
    writeVersion(next);
  }
  for (const line of lines) {
    console.log(line);
  }
  if (process.env.GITHUB_OUTPUT) {
    appendFileSync(process.env.GITHUB_OUTPUT, `${lines.join("\n")}\n`);
  }
}

if (import.meta.main) {
  main();
}
