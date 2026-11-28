import { describe, expect, it } from "vitest";
import type { Step, Turn } from "@/features/chat/api";
import {
  advanceFigures,
  buildTree,
  exitMs,
  figuresOut,
  figuresPerAgent,
  groupStatusLabel,
  subagentLabel,
  type TrayFigure,
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
    expect(exitMs("dissolving")).toBe(500);
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
