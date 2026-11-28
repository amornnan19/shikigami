import type { Step, StepStatus, Turn } from "@/features/chat/api";

export const SUBAGENT_KINDS = ["Agent", "Task"];
export const MAX_FIGURES = 3;
export const FOLD_OUT_MS = 450;
export const FOLD_BACK_MS = 350;
export const DISSOLVE_MS = 650;

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

export function withEffectiveStatus(steps: Step[]): Step[] {
  const parents = new Map(steps.map((step) => [step.id, step.parentStepId]));
  const busy = new Set<string>();
  for (const step of steps) {
    if (step.status !== "running") continue;
    let at = step.parentStepId;
    while (at && !busy.has(at)) {
      busy.add(at);
      at = parents.get(at) ?? null;
    }
  }
  return steps.map((step) =>
    isSubagent(step) && step.status !== "running" && busy.has(step.id)
      ? { ...step, status: "running" }
      : step,
  );
}

export function subagentSteps(steps: Step[]): Step[] {
  return withEffectiveStatus(steps).filter(isSubagent);
}

export function figuresOut(steps: Step[]): number {
  return subagentSteps(steps).filter((step) => step.status === "running").length;
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

export function inBackground(step: Step) {
  return step.status === "running" && step.result !== null;
}

export function groupStatusLabel(status: StepStatus, background = false): string {
  switch (status) {
    case "running":
      return background ? "In background" : "Working";
    case "done":
      return "Done";
    case "failed":
      return "Failed";
    case "stopped":
      return "Stopped";
  }
}

export function stepsSummary(steps: Step[]): string {
  const agents = steps.filter(isSubagent).length;
  const others = steps.length - agents;
  const plural = (count: number, word: string) => `${count} ${word}${count === 1 ? "" : "s"}`;
  if (agents === 0) return plural(others, "step");
  const first = plural(agents, "subagent");
  return others === 0 ? first : `${first} · ${plural(others, "step")}`;
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

export class FigureTimeline {
  private figures: TrayFigure[] = [];
  private readonly timers = new Map<string, ReturnType<typeof setTimeout>>();
  private disposed = false;

  constructor(private readonly onChange: (figures: TrayFigure[]) => void) {}

  update(subagents: Step[]) {
    if (this.disposed) return;
    this.figures = advanceFigures(this.figures, subagents);
    for (const figure of this.figures) {
      if (figure.phase === "working" || this.timers.has(figure.id)) continue;
      this.timers.set(
        figure.id,
        setTimeout(() => {
          this.timers.delete(figure.id);
          this.figures = this.figures.filter((candidate) => candidate.id !== figure.id);
          this.onChange(this.figures);
        }, exitMs(figure.phase)),
      );
    }
    this.onChange(this.figures);
  }

  dispose() {
    this.disposed = true;
    for (const timer of this.timers.values()) clearTimeout(timer);
    this.timers.clear();
  }
}
