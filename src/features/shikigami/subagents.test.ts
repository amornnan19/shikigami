import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Step, Turn } from "@/features/chat/api";
import {
  advanceFigures,
  buildTree,
  exitMs,
  FigureTimeline,
  figuresOut,
  figuresPerAgent,
  groupStatusLabel,
  inBackground,
  stepsSummary,
  subagentLabel,
  type TrayFigure,
  withEffectiveStatus,
} from "./subagents";

function step(id: string, extra: Partial<Step> = {}): Step {
  return {
    type: "step",
    id,
    parentStepId: null,
    kind: "Bash",
    title: id,
    detail: null,
    status: "running",
    task: null,
    result: null,
    ...extra,
  };
}

function agentStep(id: string, status: Step["status"], extra: Partial<Step> = {}): Step {
  return step(id, { kind: "Agent", status, ...extra });
}

describe("buildTree", () => {
  it("nests steps under the step that started them, in order", () => {
    const tree = buildTree([
      agentStep("a", "running"),
      step("a1", { parentStepId: "a" }),
      step("a2", { parentStepId: "a" }),
      step("top"),
    ]);
    expect(tree.map((node) => node.step.id)).toEqual(["a", "top"]);
    expect(tree[0]?.children.map((node) => node.step.id)).toEqual(["a1", "a2"]);
  });

  it("nests a subagent inside a subagent", () => {
    const tree = buildTree([
      agentStep("outer", "running"),
      agentStep("inner", "running", { parentStepId: "outer" }),
      step("leaf", { parentStepId: "inner" }),
    ]);
    expect(tree).toHaveLength(1);
    expect(tree[0]?.children[0]?.children[0]?.step.id).toBe("leaf");
  });

  it("keeps a step whose parent is missing at the top level", () => {
    const tree = buildTree([step("orphan", { parentStepId: "gone" })]);
    expect(tree.map((node) => node.step.id)).toEqual(["orphan"]);
  });
});

describe("subagentLabel", () => {
  it("reads the type and the task from the start of the saved input", () => {
    const detail =
      '{"subagent_type":"Explore","description":"Look \\"around\\"","prompt":"x"}\n\nfound it';
    expect(subagentLabel(agentStep("a", "done", { detail }))).toEqual({
      type: "Explore",
      task: 'Look "around"',
    });
  });

  it("reads a label from input that was cut short in the prompt", () => {
    const detail = `{"subagent_type":"Plan","description":"Draft","prompt":"${"p".repeat(50)}…`;
    expect(subagentLabel(agentStep("a", "running", { detail }))).toEqual({
      type: "Plan",
      task: "Draft",
    });
  });

  it("falls back to the tool name and the step title when the input is unreadable", () => {
    for (const detail of [null, "", "not json", '{"x":1}']) {
      expect(subagentLabel(agentStep("a", "running", { detail, title: "Look" }))).toEqual({
        type: "Agent",
        task: "Look",
      });
    }
  });

  it("falls back to the step title when the task is empty", () => {
    const detail = '{"subagent_type":"general-purpose","description":"","prompt":""}';
    expect(subagentLabel(agentStep("a", "running", { detail, title: "general-purpose" }))).toEqual({
      type: "general-purpose",
      task: "general-purpose",
    });
  });
});

describe("figures", () => {
  it("counts only running subagent steps", () => {
    expect(
      figuresOut([
        agentStep("a", "running"),
        agentStep("b", "done"),
        step("c"),
        agentStep("d", "running"),
      ]),
    ).toBe(2);
  });

  it("gives every running subagent its own working figure", () => {
    const figures = advanceFigures(
      [],
      [
        agentStep("a", "running", { detail: '{"subagent_type":"Explore","description":"x"' }),
        agentStep("b", "running"),
      ],
    );
    expect(figures).toEqual([
      { id: "a", type: "Explore", phase: "working" },
      { id: "b", type: "Agent", phase: "working" },
    ]);
  });

  it("folds a finished subagent back and dissolves a failed one", () => {
    const working: TrayFigure[] = [
      { id: "a", type: "Explore", phase: "working" },
      { id: "b", type: "Plan", phase: "working" },
      { id: "c", type: "Plan", phase: "working" },
    ];
    const next = advanceFigures(working, [
      agentStep("a", "done"),
      agentStep("b", "failed"),
      agentStep("c", "running"),
    ]);
    expect(next.map((figure) => figure.phase)).toEqual(["folding", "dissolving", "working"]);
  });

  it("treats a stopped subagent as folding back", () => {
    const next = advanceFigures(
      [{ id: "a", type: "x", phase: "working" }],
      [agentStep("a", "stopped")],
    );
    expect(next[0]?.phase).toBe("folding");
  });

  it("does not give a figure to a subagent that was already finished when first seen", () => {
    expect(advanceFigures([], [agentStep("a", "done"), agentStep("b", "failed")])).toEqual([]);
  });

  it("keeps an exiting figure exiting and never revives it", () => {
    const exiting: TrayFigure[] = [{ id: "a", type: "x", phase: "dissolving" }];
    expect(advanceFigures(exiting, [agentStep("a", "done")])[0]?.phase).toBe("dissolving");
  });

  it("uses the animation length of the phase to leave", () => {
    expect(exitMs("folding")).toBe(350);
    expect(exitMs("dissolving")).toBe(650);
  });
});

describe("group status and the room count", () => {
  it("names the status of a group", () => {
    expect(groupStatusLabel("running")).toBe("Working");
    expect(groupStatusLabel("done")).toBe("Done");
    expect(groupStatusLabel("failed")).toBe("Failed");
  });

  it("counts figures out per agent, from running turns only", () => {
    const turn = (id: string, agentId: string, status: Turn["status"], items: Step[]): Turn => ({
      id,
      agentId,
      status,
      error: null,
      startedAt: 1,
      endedAt: null,
      kind: "user",
      fromAgentId: null,
      handOffs: [],
      userMessage: "",
      items,
    });
    expect(
      figuresPerAgent([
        turn("t1", "a", "running", [agentStep("x", "running"), agentStep("y", "running")]),
        turn("t2", "b", "running", [agentStep("z", "done")]),
        turn("t3", "c", "completed", [agentStep("w", "running")]),
      ]),
    ).toEqual({ a: 2 });
  });
});

describe("effective status", () => {
  it("keeps a finished subagent running while any step below it runs", () => {
    const steps = [
      agentStep("outer", "done"),
      agentStep("inner", "done", { parentStepId: "outer" }),
      step("leaf", { parentStepId: "inner" }),
      agentStep("other", "done"),
    ];
    expect(withEffectiveStatus(steps).map((candidate) => candidate.status)).toEqual([
      "running",
      "running",
      "running",
      "done",
    ]);
    expect(figuresOut(steps)).toBe(2);
  });

  it("does not turn a plain tool step into a running one", () => {
    const steps = [step("t", { status: "done" }), step("c", { parentStepId: "t" })];
    expect(withEffectiveStatus(steps)[0]?.status).toBe("done");
  });

  it("calls a running subagent that already has its launch result in the background", () => {
    expect(inBackground(agentStep("a", "running", { result: "launched" }))).toBe(true);
    expect(inBackground(agentStep("a", "running"))).toBe(false);
    expect(inBackground(agentStep("a", "done", { result: "x" }))).toBe(false);
    expect(groupStatusLabel("running", true)).toBe("In background");
  });
});

describe("stepsSummary", () => {
  it("counts subagents apart from the other steps", () => {
    expect(
      stepsSummary([agentStep("a", "done"), agentStep("b", "done"), step("c"), step("d")]),
    ).toBe("2 subagents · 2 steps");
    expect(stepsSummary([agentStep("a", "done")])).toBe("1 subagent");
    expect(stepsSummary([step("a")])).toBe("1 step");
    expect(stepsSummary([step("a"), step("b")])).toBe("2 steps");
  });
});

describe("FigureTimeline", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("folds a finished figure back before removing it", () => {
    const seen: TrayFigure[][] = [];
    const timeline = new FigureTimeline((figures) => seen.push(figures));
    timeline.update([agentStep("a", "running")]);
    expect(seen.at(-1)?.map((figure) => figure.phase)).toEqual(["working"]);
    timeline.update([agentStep("a", "done")]);
    expect(seen.at(-1)?.map((figure) => figure.phase)).toEqual(["folding"]);
    vi.advanceTimersByTime(349);
    expect(seen.at(-1)?.map((figure) => figure.phase)).toEqual(["folding"]);
    vi.advanceTimersByTime(2);
    expect(seen.at(-1)).toEqual([]);
    timeline.dispose();
  });

  it("holds a failed figure for the red tint and the dissolve", () => {
    const seen: TrayFigure[][] = [];
    const timeline = new FigureTimeline((figures) => seen.push(figures));
    timeline.update([agentStep("a", "running")]);
    timeline.update([agentStep("a", "failed")]);
    expect(seen.at(-1)?.map((figure) => figure.phase)).toEqual(["dissolving"]);
    vi.advanceTimersByTime(649);
    expect(seen.at(-1)).toHaveLength(1);
    vi.advanceTimersByTime(2);
    expect(seen.at(-1)).toEqual([]);
    timeline.dispose();
  });

  it("does not restart the wait of a figure that is already leaving", () => {
    const seen: TrayFigure[][] = [];
    const timeline = new FigureTimeline((figures) => seen.push(figures));
    timeline.update([agentStep("a", "running"), agentStep("b", "running")]);
    timeline.update([agentStep("a", "done"), agentStep("b", "running")]);
    vi.advanceTimersByTime(300);
    timeline.update([agentStep("a", "done"), agentStep("b", "running")]);
    vi.advanceTimersByTime(60);
    expect(seen.at(-1)?.map((figure) => figure.id)).toEqual(["b"]);
    timeline.dispose();
  });

  it("clears every timer on dispose and reports nothing afterwards", () => {
    const onChange = vi.fn();
    const timeline = new FigureTimeline(onChange);
    timeline.update([agentStep("a", "running")]);
    timeline.update([agentStep("a", "done")]);
    expect(vi.getTimerCount()).toBe(1);
    const calls = onChange.mock.calls.length;
    timeline.dispose();
    expect(vi.getTimerCount()).toBe(0);
    vi.advanceTimersByTime(1000);
    timeline.update([agentStep("b", "running")]);
    expect(onChange.mock.calls.length).toBe(calls);
  });
});
