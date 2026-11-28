import { describe, expect, it } from "vitest";
import type { Conversation, TurnEvent } from "./api";
import { applyEvent, mergeNewest, prependOlder } from "./conversation";

function turn(id: string): Conversation["turns"][number] {
  return {
    id,
    agentId: "a",
    status: "running",
    error: null,
    startedAt: 1,
    endedAt: null,
    kind: "user",
    fromAgentId: null,
    handOffs: [],
    userMessage: id,
    items: [],
  };
}

const base: Conversation = { turns: [turn("t1")], hasMore: false };

function apply(data: Conversation, event: object) {
  return applyEvent(data, { agentId: "a", groupId: null, turnId: "t1", ...event } as TurnEvent);
}

describe("applyEvent", () => {
  it("appends deltas to the item and creates it when unknown", () => {
    const first = apply(base, { kind: "text-delta", itemId: "i", text: "po" });
    const second = apply(first, { kind: "text-delta", itemId: "i", text: "ng" });
    expect(second.turns[0]?.items).toEqual([{ type: "text", id: "i", text: "pong" }]);
  });

  it("replaces the text on text-set", () => {
    const streamed = apply(base, { kind: "text-delta", itemId: "i", text: "pon" });
    const set = apply(streamed, { kind: "text-set", itemId: "i", text: "pong" });
    expect(set.turns[0]?.items).toEqual([{ type: "text", id: "i", text: "pong" }]);
  });

  it("starts and ends a step, and stops open steps when the turn ends", () => {
    const step = {
      id: "s",
      parentStepId: null,
      kind: "Bash",
      title: "ls",
      detail: "{}",
      status: "running",
    };
    const started = apply(base, { kind: "step-started", step });
    const ended = apply(started, { kind: "step-ended", stepId: "s", status: "done", detail: "x" });
    expect(ended.turns[0]?.items[0]).toMatchObject({ status: "done", detail: "x" });
    const open = apply(started, { kind: "ended", status: "failed", error: "boom" });
    expect(open.turns[0]).toMatchObject({ status: "failed", error: "boom" });
    expect(open.turns[0]?.items[0]).toMatchObject({ status: "stopped" });
  });

  it("ignores events for turns it does not have", () => {
    const other = applyEvent(base, {
      agentId: "a",
      groupId: null,
      turnId: "zzz",
      kind: "text-delta",
      itemId: "i",
      text: "x",
    });
    expect(other).toBe(base);
  });
});

describe("notices", () => {
  it("shows a notice once when its event follows a refetch that already has it", () => {
    const notice = { kind: "notice", itemId: "n", text: "New session" };
    const once = apply(base, notice);
    const twice = apply(once, notice);
    expect(twice.turns[0]?.items).toEqual([{ type: "notice", id: "n", text: "New session" }]);
  });
});

describe("refetch during a stream", () => {
  const running = (text: string): Conversation => ({
    turns: [{ ...turn("t1"), items: [{ type: "text", id: "i", text }] }],
    hasMore: false,
  });

  it("keeps the longer streamed text of a running turn", () => {
    const merged = mergeNewest(running("pong and more"), running("pong"));
    expect(merged.turns[0]?.items).toEqual([{ type: "text", id: "i", text: "pong and more" }]);
  });

  it("takes the fresh text when it is longer", () => {
    const merged = mergeNewest(running("po"), running("pong"));
    expect(merged.turns[0]?.items).toEqual([{ type: "text", id: "i", text: "pong" }]);
  });

  it("takes the saved text of a turn that has ended", () => {
    const ended = running("pong");
    const first = ended.turns[0];
    if (!first) throw new Error("no turn");
    ended.turns[0] = { ...first, status: "completed" };
    const merged = mergeNewest(running("pong and more"), ended);
    expect(merged.turns[0]?.items).toEqual([{ type: "text", id: "i", text: "pong" }]);
  });
});

describe("paging", () => {
  it("keeps older turns that a refetch of the newest page does not return", () => {
    const previous = { turns: [turn("t1"), turn("t2"), turn("t3")], hasMore: true };
    const fresh = { turns: [turn("t2"), turn("t3"), turn("t4")], hasMore: true };
    const merged = mergeNewest(previous, fresh);
    expect(merged.turns.map((t) => t.id)).toEqual(["t1", "t2", "t3", "t4"]);
    expect(merged.hasMore).toBe(true);
  });

  it("uses the fresh page when nothing overlaps and prepends older pages once", () => {
    const fresh = { turns: [turn("t9")], hasMore: false };
    expect(mergeNewest({ turns: [turn("t1")], hasMore: true }, fresh)).toBe(fresh);
    const merged = prependOlder(
      { turns: [turn("t2")], hasMore: true },
      { turns: [turn("t0"), turn("t1"), turn("t2")], hasMore: false },
    );
    expect(merged.turns.map((t) => t.id)).toEqual(["t0", "t1", "t2"]);
    expect(merged.hasMore).toBe(false);
  });
});
