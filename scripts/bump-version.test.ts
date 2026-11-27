import { describe, expect, it } from "vitest";
import { type Commit, decideBump, nextVersion, parseCommit } from "./bump-version";

const commit = (subject: string, body = ""): Commit => ({ subject, body });

describe("parseCommit", () => {
  it("reads the type of a plain subject", () => {
    expect(parseCommit("feat: add x", "")).toEqual({ type: "feat", breaking: false });
  });

  it("reads the type of a scoped subject", () => {
    expect(parseCommit("fix(ui): repair x", "")).toEqual({ type: "fix", breaking: false });
  });

  it("marks a bang as breaking", () => {
    expect(parseCommit("feat!: change x", "")).toEqual({ type: "feat", breaking: true });
  });

  it("marks a BREAKING CHANGE footer as breaking", () => {
    const body = "Why.\n\nBREAKING CHANGE: x is gone";
    expect(parseCommit("refactor: drop x", body)).toEqual({ type: "refactor", breaking: true });
  });

  it("marks a BREAKING-CHANGE footer as breaking", () => {
    expect(parseCommit("fix: y", "BREAKING-CHANGE: y").breaking).toBe(true);
  });

  it("gives other for a subject that is not conventional", () => {
    expect(parseCommit("Merge branch 'main' into x", "")).toEqual({
      type: "other",
      breaking: false,
    });
  });
});

describe("decideBump", () => {
  it("gives null for types that do not bump", () => {
    const commits = ["docs: a", "chore: b", "test: c", "ci: d", "refactor: e", "style: f"].map(
      (subject) => commit(subject),
    );
    expect(decideBump(commits, "0.1.0")).toBeNull();
  });

  it("gives null for an empty list", () => {
    expect(decideBump([], "0.1.0")).toBeNull();
  });

  it("gives patch for a fix", () => {
    expect(decideBump([commit("docs: a"), commit("fix: b")], "0.1.0")).toBe("patch");
  });

  it("gives patch for perf", () => {
    expect(decideBump([commit("perf: a")], "0.1.0")).toBe("patch");
  });

  it("gives minor for a fix with a feat", () => {
    expect(decideBump([commit("fix: a"), commit("feat: b")], "0.1.0")).toBe("minor");
  });

  it("gives minor for a breaking change before 1.0.0", () => {
    expect(decideBump([commit("feat!: a")], "0.4.2")).toBe("minor");
  });

  it("gives major for a breaking change from 1.0.0", () => {
    expect(decideBump([commit("fix: a", "BREAKING CHANGE: b")], "1.2.3")).toBe("major");
  });

  it("ignores release commits", () => {
    expect(decideBump([commit("chore(release): v0.2.0")], "0.2.0")).toBeNull();
  });
});

describe("nextVersion", () => {
  it("raises patch", () => {
    expect(nextVersion("0.1.0", "patch")).toBe("0.1.1");
  });

  it("raises minor and resets patch", () => {
    expect(nextVersion("0.1.4", "minor")).toBe("0.2.0");
  });

  it("raises major and resets minor and patch", () => {
    expect(nextVersion("1.2.3", "major")).toBe("2.0.0");
  });

  it("throws for a version that is not semver", () => {
    expect(() => nextVersion("1.2", "patch")).toThrow();
    expect(() => nextVersion("1.2.3-beta.1", "patch")).toThrow();
  });
});
