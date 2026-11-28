import {
  ArrowRight,
  Check,
  ChevronDown,
  ChevronRight,
  Clock,
  LoaderCircle,
  Square,
  X,
} from "lucide-react";
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { AgentAvatar } from "@/features/agents/AgentAvatar";
import { tintClass } from "@/features/agents/colors";
import type { GroupMember } from "@/features/groups/api";
import { FigureTray } from "@/features/shikigami/FigureTray";
import { PaperFigure } from "@/features/shikigami/PaperFigure";
import {
  buildTree,
  groupStatusLabel,
  inBackground,
  isSubagent,
  type StepNode,
  stepsSummary,
  subagentLabel,
  withEffectiveStatus,
} from "@/features/shikigami/subagents";
import { cn } from "@/lib/utils";
import { ApprovalCard } from "./ApprovalCard";
import type { Approval, Step, StepStatus, TextItem, Turn } from "./api";
import { Markdown, MentionChip, withMentions } from "./Markdown";

const STATUS_LABEL: Record<StepStatus, string> = {
  running: "Running",
  done: "Done",
  failed: "Failed",
  stopped: "Stopped",
};

function StepIcon({ status }: { status: StepStatus }) {
  const className = "mt-0.5 size-3.5 shrink-0";
  return (
    <span role="img" aria-label={STATUS_LABEL[status]}>
      {status === "running" && <LoaderCircle className={cn(className, "animate-spin")} />}
      {status === "done" && <Check className={cn(className, "text-green-600")} />}
      {status === "failed" && <X className={cn(className, "text-red-600")} />}
      {status === "stopped" && <Square className={cn(className, "text-muted-foreground")} />}
    </span>
  );
}

function StepDetail({ step }: { step: Step }) {
  if (!step.detail) return null;
  return (
    <pre className="mt-1 max-h-60 overflow-auto rounded-md border border-border bg-muted p-2 font-mono text-xs whitespace-pre-wrap break-all">
      {step.detail}
    </pre>
  );
}

function StepRows({ nodes, tint, busy }: { nodes: StepNode[]; tint: string; busy: Set<string> }) {
  return (
    <ul className="mt-1 flex flex-col gap-2">
      {nodes.map((node) =>
        isSubagent(node.step) ? (
          <SubagentGroup
            key={node.step.id}
            node={node}
            tint={tint}
            running={busy.has(node.step.id)}
            busy={busy}
          />
        ) : (
          <li key={node.step.id} className="min-w-0">
            <div className="flex items-start gap-2">
              <StepIcon status={node.step.status} />
              <span className="font-medium text-foreground">{node.step.kind}</span>
              <span className="min-w-0 truncate">{node.step.title}</span>
            </div>
            <StepDetail step={node.step} />
          </li>
        ),
      )}
    </ul>
  );
}

function Clamped({ label, text }: { label: string; text: string }) {
  const [expanded, setExpanded] = useState(false);
  const [overflows, setOverflows] = useState(false);
  const box = useRef<HTMLParagraphElement>(null);
  // biome-ignore lint/correctness/useExhaustiveDependencies: measure again when the text changes
  useLayoutEffect(() => {
    const element = box.current;
    if (element && !expanded) setOverflows(element.scrollHeight > element.clientHeight + 1);
  }, [text, expanded]);
  return (
    <div data-subagent-task="" className="mb-2">
      <span className="text-xs font-medium text-foreground">{label}</span>
      <p ref={box} className={cn("break-words whitespace-normal", !expanded && "line-clamp-3")}>
        {text}
      </p>
      {(overflows || expanded) && (
        <button
          type="button"
          onClick={() => setExpanded(!expanded)}
          className="text-xs underline outline-none focus-visible:ring-2 focus-visible:ring-ring"
        >
          {expanded ? "Show less" : "Show more"}
        </button>
      )}
    </div>
  );
}

function SubagentGroup({
  node,
  tint,
  running,
  busy,
}: {
  node: StepNode;
  tint: string;
  running: boolean;
  busy: Set<string>;
}) {
  const { step } = node;
  const [chosen, setChosen] = useState<boolean | null>(null);
  const status: StepStatus = running ? "running" : step.status;
  const open = chosen ?? status === "running";
  const Chevron = open ? ChevronDown : ChevronRight;
  const label = subagentLabel(step);
  return (
    <li data-subagent={step.id} data-status={status} className="min-w-0">
      <button
        type="button"
        aria-expanded={open}
        onClick={() => setChosen(!open)}
        className="flex w-full items-start gap-2 rounded-md px-1 py-0.5 text-left outline-none hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring"
      >
        <Chevron className="mt-0.5 size-3.5 shrink-0" />
        <span aria-hidden="true" className="mt-px block h-[18px] w-3.5 shrink-0">
          <PaperFigure tintClass={step.status === "failed" ? "text-red-600" : tint} />
        </span>
        <span className="font-medium text-foreground">{label.type}</span>
        <span className="min-w-0 flex-1 truncate">{label.task}</span>
        <span
          data-subagent-status=""
          className={cn("shrink-0 text-xs", step.status === "failed" && "text-red-600")}
        >
          {groupStatusLabel(status, inBackground(step))}
        </span>
      </button>
      {open && (
        <div className="mt-1 ml-2 border-l border-border pl-3">
          {step.task && <Clamped label="Task" text={step.task} />}
          {node.children.length > 0 && <StepRows nodes={node.children} tint={tint} busy={busy} />}
          {step.result && (
            <div data-subagent-result="" className="mt-2">
              <span className="text-xs font-medium text-foreground">Result</span>
              <p className="max-h-60 overflow-auto break-words whitespace-pre-wrap">
                {step.result}
              </p>
            </div>
          )}
        </div>
      )}
    </li>
  );
}

function Steps({ steps, tint }: { steps: Step[]; tint: string }) {
  const [chosen, setChosen] = useState<boolean | null>(null);
  const effective = withEffectiveStatus(steps);
  const busy = new Set(
    effective.filter((step) => isSubagent(step) && step.status === "running").map((s) => s.id),
  );
  const anyRunning = busy.size > 0;
  useEffect(() => {
    if (anyRunning) setChosen((current) => current ?? true);
  }, [anyRunning]);
  const open = chosen ?? false;
  const Chevron = open ? ChevronDown : ChevronRight;
  return (
    <div className="my-2 text-sm text-muted-foreground">
      <button
        type="button"
        aria-expanded={open}
        onClick={() => setChosen(!open)}
        className="flex items-center gap-1 rounded-md px-1 py-0.5 outline-none hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring"
      >
        <Chevron className="size-4" />
        {stepsSummary(steps)}
      </button>
      {open && (
        <div className="pl-5">
          <StepRows nodes={buildTree(steps)} tint={tint} busy={busy} />
        </div>
      )}
    </div>
  );
}

const FORMER_MEMBER: Pick<GroupMember, "name" | "color"> = {
  name: "Former member",
  color: "slate",
};

function HandOffMarkers({ turn, members }: { turn: Turn; members?: GroupMember[] }) {
  if (!members) return null;
  return (
    <>
      {turn.handOffs.map((handOff, index) => {
        const member = members.find((candidate) => candidate.agentId === handOff.agentId);
        if (!member) return null;
        return (
          <p
            // biome-ignore lint/suspicious/noArrayIndexKey: a turn can hand the same task to one member twice
            key={index}
            data-hand-off={member.agentId}
            className="my-1 flex items-start gap-1.5 text-xs text-muted-foreground"
          >
            <ArrowRight className="mt-0.5 size-3.5 shrink-0" />
            <span className="min-w-0">
              Handed to <MentionChip member={member} />
              <span className="ml-1 break-words">{handOff.task}</span>
            </span>
          </p>
        );
      })}
    </>
  );
}

export function TurnView({
  turn,
  members,
  color,
  name,
}: {
  turn: Turn;
  members?: GroupMember[];
  color?: string;
  name?: string;
}) {
  const notices = turn.items.filter((item): item is TextItem => item.type === "notice");
  const steps = turn.items.filter((item): item is Step => item.type === "step");
  const texts = turn.items.filter((item): item is TextItem => item.type === "text");
  const approvals = turn.items.filter((item): item is Approval => item.type === "approval");
  const author = members
    ? (members.find((member) => member.agentId === turn.agentId) ?? FORMER_MEMBER)
    : null;
  const reply = (
    <>
      {turn.items.map((item) => {
        if (item.type === "step") {
          return item.id === steps[0]?.id ? (
            <Steps key="steps" steps={steps} tint={tintClass(author?.color ?? color ?? "slate")} />
          ) : null;
        }
        if (item.type === "text") {
          return <Markdown key={item.id} text={item.text} members={members} />;
        }
        if (item.type === "approval") {
          return (
            <ApprovalCard
              key={item.id}
              approval={item}
              name={author?.name ?? name ?? "Agent"}
              color={author?.color ?? color ?? "slate"}
              showName={!author}
            />
          );
        }
        return null;
      })}
      <HandOffMarkers turn={turn} members={members} />
      {turn.status === "running" &&
        texts.length === 0 &&
        steps.length === 0 &&
        approvals.length === 0 && (
          <p role="status" className="animate-pulse text-sm text-muted-foreground">
            Working…
          </p>
        )}
      {turn.error && <p className="text-sm text-red-600/80">{turn.error}</p>}
      {turn.status === "stopped" && <p className="text-sm text-muted-foreground">Stopped</p>}
      {turn.status === "interrupted" && (
        <p className="text-sm text-muted-foreground">This reply was interrupted.</p>
      )}
    </>
  );
  const sender = members?.find((member) => member.agentId === turn.fromAgentId);
  return (
    <article data-turn-id={turn.id} className="flex flex-col gap-2">
      {turn.routineName !== null && (
        <p
          data-part="routine-marker"
          className="ml-auto flex items-center gap-1 text-xs text-muted-foreground"
        >
          <Clock className="size-3" aria-hidden="true" />
          Routine: {turn.routineName}
        </p>
      )}
      {turn.kind === "continuation" ? (
        <p className="my-1 text-center text-xs text-muted-foreground">{turn.userMessage}</p>
      ) : turn.kind === "hand-off" ? (
        <div className="max-w-[80%] rounded-2xl border border-border bg-muted px-4 py-2 text-sm whitespace-pre-wrap break-words">
          <span className="block text-xs text-muted-foreground">
            Task from {sender?.name ?? FORMER_MEMBER.name}
          </span>
          {members ? withMentions(turn.userMessage, members) : turn.userMessage}
        </div>
      ) : turn.userMessage === "" ? null : (
        <div className="ml-auto max-w-[80%] rounded-2xl bg-primary px-4 py-2 whitespace-pre-wrap break-words text-primary-foreground">
          {members ? withMentions(turn.userMessage, members) : turn.userMessage}
        </div>
      )}
      {notices.map((item) => (
        <p key={item.id} className="my-1 text-center text-xs text-muted-foreground">
          {item.text}
        </p>
      ))}
      {author ? (
        <div className="flex gap-3">
          <span data-figure-anchor="" className="relative shrink-0">
            <FigureTray steps={steps} color={author.color} anchor={32} />
            <span className="relative z-10 block">
              <AgentAvatar name={author.name} color={author.color} className="size-8" />
            </span>
          </span>
          <div className="flex min-w-0 flex-1 flex-col gap-2">
            <span className="text-sm font-medium">{author.name}</span>
            {reply}
          </div>
        </div>
      ) : (
        reply
      )}
    </article>
  );
}
