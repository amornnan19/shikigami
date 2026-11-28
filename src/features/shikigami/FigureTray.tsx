import { useQuery } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { type CSSProperties, useEffect, useRef, useState, useSyncExternalStore } from "react";
import { tintClass } from "@/features/agents/colors";
import type { Step } from "@/features/chat/api";
import { cn } from "@/lib/utils";
import { PaperFigure } from "./PaperFigure";
import {
  FigureTimeline,
  figuresOut,
  MAX_FIGURES,
  subagentSteps,
  type TrayFigure,
} from "./subagents";

const REDUCED_QUERY = "(prefers-reduced-motion: reduce)";
const SLOT_PX = 20;
const FIGURE_WIDTH_PX = 24;
const TRAY_LIFT_PX = 12;
const TRAY_LEFT_PX = 4;
const FIGURE_HEIGHT_PX = 32;

function subscribeReduced(notify: () => void) {
  const media = window.matchMedia(REDUCED_QUERY);
  media.addEventListener("change", notify);
  return () => media.removeEventListener("change", notify);
}

export function useReducedMotion() {
  const system = useSyncExternalStore(
    subscribeReduced,
    () => window.matchMedia(REDUCED_QUERY).matches,
    () => false,
  );
  const override = useQuery({
    queryKey: ["reduce-motion-override"],
    queryFn: () => invoke<boolean>("reduce_motion_override"),
    staleTime: Number.POSITIVE_INFINITY,
    retry: false,
  });
  return system || override.data === true;
}

export function useTrayFigures(steps: Step[]) {
  const subagents = subagentSteps(steps);
  const signature = subagents.map((step) => `${step.id}:${step.status}`).join(",");
  const [figures, setFigures] = useState<TrayFigure[]>([]);
  const timeline = useRef<FigureTimeline | null>(null);
  useEffect(() => {
    const created = new FigureTimeline(setFigures);
    timeline.current = created;
    return () => {
      created.dispose();
      timeline.current = null;
    };
  }, []);
  // biome-ignore lint/correctness/useExhaustiveDependencies: the signature stands for the subagent steps
  useEffect(() => {
    timeline.current?.update(subagents);
  }, [signature]);
  return figures;
}

export function FigureTrayView({
  figures,
  running,
  tint,
  reduced,
  anchor,
}: {
  figures: TrayFigure[];
  running: number;
  tint: string;
  reduced: boolean;
  anchor: number;
}) {
  if (reduced) {
    if (running === 0) return null;
    const label = running === 1 ? "1 paper figure out" : `${running} paper figures out`;
    return (
      <span
        role="img"
        aria-label={label}
        title={label}
        data-figure-badge=""
        style={{ left: TRAY_LEFT_PX, top: -TRAY_LIFT_PX }}
        className="absolute z-0 block h-8 w-8"
      >
        <span
          aria-hidden="true"
          className="absolute top-0 left-2 block h-8 w-6 -rotate-6 opacity-60"
        >
          <PaperFigure tintClass={tint} />
        </span>
        <span aria-hidden="true" className="absolute top-0 left-0 block h-8 w-6">
          <PaperFigure tintClass={tint} />
        </span>
        <span
          aria-hidden="true"
          className="absolute top-0 -right-2 rounded-full bg-background px-1 text-[10px] leading-4 font-medium text-foreground ring-1 ring-border"
        >
          {running}
        </span>
      </span>
    );
  }
  if (figures.length === 0) return null;
  const shown = figures.slice(0, MAX_FIGURES);
  const extra = figures.length - shown.length;
  const fromY = anchor / 2 - (FIGURE_HEIGHT_PX / 2 - TRAY_LIFT_PX);
  return (
    <span
      data-figure-tray=""
      style={{ left: TRAY_LEFT_PX, top: -TRAY_LIFT_PX }}
      className="absolute z-0 flex"
    >
      {shown.map((figure, slot) => (
        <span
          key={figure.id}
          role="img"
          aria-label={figure.type}
          title={figure.type}
          data-figure={figure.id}
          data-phase={figure.phase}
          style={
            {
              "--from-x": `${anchor / 2 - (TRAY_LEFT_PX + slot * SLOT_PX + FIGURE_WIDTH_PX / 2)}px`,
              "--from-y": `${fromY}px`,
              marginLeft: slot === 0 ? 0 : SLOT_PX - FIGURE_WIDTH_PX,
              rotate: `${slot * 5}deg`,
            } as CSSProperties
          }
          className={cn("paper-figure relative block h-8 w-6", `paper-figure-${figure.phase}`)}
        >
          <PaperFigure tintClass={figure.phase === "dissolving" ? "text-red-600" : tint} />
          {extra > 0 && slot === shown.length - 1 && (
            <span
              data-figure-more=""
              className="absolute -right-2 -bottom-1 rounded-full bg-background px-1 text-[10px] leading-4 font-medium text-foreground ring-1 ring-border"
            >
              +{extra}
            </span>
          )}
        </span>
      ))}
    </span>
  );
}

export function FigureTray({
  steps,
  color,
  anchor,
}: {
  steps: Step[];
  color: string;
  anchor: number;
}) {
  const figures = useTrayFigures(steps);
  const reduced = useReducedMotion();
  return (
    <FigureTrayView
      figures={figures}
      running={figuresOut(steps)}
      tint={tintClass(color)}
      reduced={reduced}
      anchor={anchor}
    />
  );
}
