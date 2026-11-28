import { useQuery } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import {
  type CSSProperties,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  useSyncExternalStore,
} from "react";
import { tintClass } from "@/features/agents/colors";
import type { Step } from "@/features/chat/api";
import { cn } from "@/lib/utils";
import { PaperFigure } from "./PaperFigure";
import {
  advanceFigures,
  exitMs,
  figuresOut,
  MAX_FIGURES,
  subagentSteps,
  type TrayFigure,
} from "./subagents";

const REDUCED_QUERY = "(prefers-reduced-motion: reduce)";
const SLOT_PX = 28;

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
  const scheduled = useRef(new Set<string>());
  // biome-ignore lint/correctness/useExhaustiveDependencies: the signature stands for the subagent steps
  useEffect(() => {
    setFigures((previous) => advanceFigures(previous, subagents));
  }, [signature]);
  useEffect(() => {
    const timers: number[] = [];
    for (const figure of figures) {
      if (figure.phase === "working" || scheduled.current.has(figure.id)) continue;
      scheduled.current.add(figure.id);
      timers.push(
        window.setTimeout(() => {
          setFigures((current) => current.filter((candidate) => candidate.id !== figure.id));
        }, exitMs(figure.phase)),
      );
    }
  }, [figures]);
  return figures;
}

export function FigureTrayView({
  figures,
  running,
  tint,
  reduced,
  origin,
}: {
  figures: TrayFigure[];
  running: number;
  tint: string;
  reduced: boolean;
  origin: number;
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
        className="relative flex h-6 items-center gap-1 pr-1 text-xs text-muted-foreground"
      >
        <span aria-hidden="true" className="relative block h-5 w-6">
          <span className="absolute top-0 left-2 block h-5 w-4 -rotate-6 opacity-60">
            <PaperFigure tintClass={tint} />
          </span>
          <span className="absolute top-0 left-0 block h-5 w-4">
            <PaperFigure tintClass={tint} />
          </span>
        </span>
        <span aria-hidden="true">{running}</span>
      </span>
    );
  }
  if (figures.length === 0) return null;
  const shown = figures.slice(0, MAX_FIGURES);
  const extra = figures.length - shown.length;
  return (
    <span
      data-figure-tray=""
      style={{ "--origin": `${origin}px` } as CSSProperties}
      className="relative flex h-8 items-center gap-1"
    >
      {shown.map((figure, slot) => (
        <span
          key={figure.id}
          role="img"
          aria-label={figure.type}
          title={figure.type}
          data-figure={figure.id}
          data-phase={figure.phase}
          style={{ "--from-x": `${-(origin + slot * SLOT_PX)}px` } as CSSProperties}
          className={cn("paper-figure relative block h-7 w-6", `paper-figure-${figure.phase}`)}
        >
          <PaperFigure tintClass={figure.phase === "dissolving" ? "text-red-600" : tint} />
          {extra > 0 && slot === shown.length - 1 && (
            <span
              data-figure-more=""
              className="absolute -right-1 -bottom-1 rounded-full bg-foreground px-1 text-[10px] leading-4 text-background"
            >
              +{extra}
            </span>
          )}
        </span>
      ))}
    </span>
  );
}

export function FigureTray({ steps, color }: { steps: Step[]; color: string }) {
  const figures = useTrayFigures(steps);
  const reduced = useReducedMotion();
  const holder = useRef<HTMLSpanElement>(null);
  const [origin, setOrigin] = useState(44);
  const count = figures.length;
  // biome-ignore lint/correctness/useExhaustiveDependencies: measure again when the figures change
  useLayoutEffect(() => {
    const tray = holder.current?.querySelector("[data-figure-tray]");
    const anchor = holder.current
      ?.closest("[data-figure-scope]")
      ?.querySelector("[data-figure-anchor]");
    if (tray && anchor) {
      setOrigin(
        Math.round(tray.getBoundingClientRect().left - anchor.getBoundingClientRect().left),
      );
    }
  }, [count]);
  return (
    <span ref={holder} className="contents">
      <FigureTrayView
        figures={figures}
        running={figuresOut(steps)}
        tint={tintClass(color)}
        reduced={reduced}
        origin={origin}
      />
    </span>
  );
}
