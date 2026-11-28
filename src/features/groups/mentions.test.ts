import { describe, expect, it } from "vitest";
import vectorsText from "../../../src-tauri/tests/fixtures/mention-vectors.json?raw";
import type { GroupMember } from "./api";
import { findMentions, suggestionAt } from "./mentions";

const members: GroupMember[] = [
  { agentId: "a", name: "Ada", color: "red" },
  { agentId: "b", name: "Bo", color: "blue" },
  { agentId: "t", name: "Test", color: "green" },
  { agentId: "ta", name: "Test Agent", color: "teal" },
];

const ids = (text: string) => findMentions(text, members).map((mention) => mention.member.agentId);

describe("findMentions", () => {
  it("matches names case-insensitively and takes the longest name", () => {
    expect(ids("@bo list")).toEqual(["b"]);
    expect(ids("@test agent look")).toEqual(["ta"]);
    expect(ids("@Test look")).toEqual(["t"]);
    expect(ids("@Test Agentx look")).toEqual(["t"]);
  });

  it("needs the end of the text, whitespace or listed punctuation after the name", () => {
    expect(ids("@Bo")).toEqual(["b"]);
    for (const end of [",", ".", ":", ";", "!", "?", ")", "]"]) {
      expect(ids(`(see @Bo${end} ok`)).toEqual(["b"]);
    }
    for (const glued of ["x", "1", "-", "'", "@"]) {
      expect(ids(`@Bo${glued} ok`)).toEqual([]);
    }
    expect(ids("mail ada@Bo.com")).toEqual([]);
  });

  it("ignores fenced and inline code and non-members", () => {
    expect(ids("```\n@Bo hidden\n```")).toEqual([]);
    expect(ids("run `ask @Bo please` now")).toEqual([]);
    expect(ids("run ``a ` @Bo please`` now")).toEqual([]);
    expect(ids("```\n@Bo\n```\n@Ada visible")).toEqual(["a"]);
    expect(ids("@Zed hi")).toEqual([]);
  });

  it("reports the span of each mention in order", () => {
    const [first, second] = findMentions("hi @Bo and @Ada.", members);
    expect([first?.start, first?.end]).toEqual([3, 6]);
    expect([second?.start, second?.end]).toEqual([11, 15]);
  });
});

describe("shared vectors", () => {
  it("give the same mentions as the Rust parser", () => {
    const vectors: Array<{ text: string; members: string[]; mentions: string[] }> =
      JSON.parse(vectorsText);
    for (const vector of vectors) {
      const team = vector.members.map((name) => ({ agentId: name, name, color: "red" }));
      const found = findMentions(vector.text, team).map((mention) => mention.member.name);
      expect(found, JSON.stringify(vector.text)).toEqual(vector.mentions);
    }
  });
});

describe("suggestionAt", () => {
  it("offers members whose name starts with what follows the @ before the cursor", () => {
    expect(suggestionAt("hi @", 4, members)?.members).toHaveLength(4);
    expect(suggestionAt("hi @te", 6, members)?.members.map((member) => member.agentId)).toEqual([
      "t",
      "ta",
    ]);
    expect(suggestionAt("@test a", 7, members)?.members.map((member) => member.agentId)).toEqual([
      "ta",
    ]);
    expect(suggestionAt("@te more", 3, members)?.start).toBe(0);
  });

  it("lists the member whose name equals the query first and keeps the list open", () => {
    expect(suggestionAt("@Test", 5, members)?.members.map((member) => member.agentId)).toEqual([
      "t",
      "ta",
    ]);
    expect(suggestionAt("@Bo", 3, members)?.members.map((member) => member.agentId)).toEqual(["b"]);
  });

  it("offers nothing without a fresh @, a match or on the same line", () => {
    expect(suggestionAt("hello", 5, members)).toBeNull();
    expect(suggestionAt("mail@", 5, members)).toBeNull();
    expect(suggestionAt("@zz", 3, members)).toBeNull();
    expect(suggestionAt("@Bo ", 4, members)).toBeNull();
    expect(suggestionAt("@bo  ", 5, members)).toBeNull();
    expect(suggestionAt("@B\nx", 4, members)).toBeNull();
  });
});
