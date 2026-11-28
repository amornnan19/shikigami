import type { Step, StepStatus, Turn } from "@/features/chat/api";

export const SUBAGENT_KINDS = ["Agent", "Task"];
export const MAX_FIGURES = 3;
export const FOLD_OUT_MS = 450;
export const FOLD_BACK_MS = 350;
export const DISSOLVE_MS = 500;

export interface StepNode {
  step: Step;
  children: StepNode[];
}

export interface SubagentLabel {
  type: string;
  task: string;
}

export type FigurePhase = "working" | "folding" | "dissolving";

export interface TrayFigure {
  id: string;
  type: string;
  phase: FigurePhase;
}

export function isSubagent(step: Step) {
  return SUBAGENT_KINDS.includes(step.kind);
}

export function buildTree(steps: Step[]): StepNode[] {
  const nodes = new Map<string, StepNode>(steps.map((step) => [step.id, { step, children: [] }]));
  const roots: StepNode[] = [];
  for (const step of steps) {
    const node = nodes.get(step.id);
    if (!node) continue;
    const parent = step.parentStepId ? nodes.get(step.parentStepId) : undefined;
    if (parent && parent !== node) {
      parent.children.push(node);
    } else {
      roots.push(node);
    }
  }
  return roots;
}

const LEADING_FIELDS = /^\{"subagent_type":("(?:[^"\\]|\\.)*"),"description":("(?:[^"\\]|\\.)*")/;

export function subagentLabel(step: Step): SubagentLabel {
  const found = step.detail ? LEADING_FIELDS.exec(step.detail) : null;
  if (found?.[1] !== undefined && found[2] !== undefined) {
    try {
      const type = JSON.parse(found[1]) as string;
      const task = JSON.parse(found[2]) as string;
      return { type: type || step.kind, task: task || step.title };
    } catch {
      return { type: step.kind, task: step.title };
    }
  }
  return { type: step.kind, task: step.title };
}

export function subagentSteps(steps: Step[]): Step[] {
  return steps.filter(isSubagent);
}

export function figuresOut(steps: Step[]): number {
  return steps.filter((step) => isSubagent(step) && step.status === "running").length;
}

export function figuresPerAgent(turns: Turn[]): Record<string, number> {
  const counts: Record<string, number> = {};
  for (const turn of turns) {
    if (turn.status !== "running" || turn.agentId === null) continue;
    const steps = turn.items.filter((item): item is Step => item.type === "step");
    const out = figuresOut(steps);
    if (out > 0) counts[turn.agentId] = (counts[turn.agentId] ?? 0) + out;
  }
  return counts;
}

export function groupStatusLabel(status: StepStatus): string {
  switch (status) {
    case "running":
      return "Working";
    case "done":
      return "Done";
    case "failed":
      return "Failed";
    case "stopped":
      return "Stopped";
  }
}

export function advanceFigures(previous: TrayFigure[], subagents: Step[]): TrayFigure[] {
  const known = new Map(previous.map((figure) => [figure.id, figure]));
  const next: TrayFigure[] = [];
  for (const step of subagents) {
    const existing = known.get(step.id);
    if (step.status === "running") {
      next.push(existing ?? { id: step.id, type: subagentLabel(step).type, phase: "working" });
    } else if (existing) {
      const phase: FigurePhase =
        existing.phase !== "working"
          ? existing.phase
          : step.status === "failed"
            ? "dissolving"
            : "folding";
      next.push({ ...existing, phase });
    }
  }
  return next;
}

export function exitMs(phase: FigurePhase) {
  return phase === "dissolving" ? DISSOLVE_MS : FOLD_BACK_MS;
}
