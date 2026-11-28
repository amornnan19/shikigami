import {
  ArrowRight,
  Check,
  ChevronDown,
  ChevronRight,
  LoaderCircle,
  Square,
  X,
} from "lucide-react";
import { useState } from "react";
import { AgentAvatar } from "@/features/agents/AgentAvatar";
import { tintClass } from "@/features/agents/colors";
import type { GroupMember } from "@/features/groups/api";
import { FigureTray } from "@/features/shikigami/FigureTray";
import { PaperFigure } from "@/features/shikigami/PaperFigure";
import {
  buildTree,
  groupStatusLabel,
  isSubagent,
  type StepNode,
  subagentLabel,
} from "@/features/shikigami/subagents";
import { cn } from "@/lib/utils";
import type { Step, StepStatus, TextItem, Turn } from "./api";
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

function StepRows({ nodes, tint }: { nodes: StepNode[]; tint: string }) {
  return (
    <ul className="mt-1 flex flex-col gap-2">
      {nodes.map((node) =>
        isSubagent(node.step) ? (
          <SubagentGroup key={node.step.id} node={node} tint={tint} />
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

function SubagentGroup({ node, tint }: { node: StepNode; tint: string }) {
  const { step } = node;
  const [chosen, setChosen] = useState<boolean | null>(null);
  const open = chosen ?? step.status === "running";
  const Chevron = open ? ChevronDown : ChevronRight;
  const label = subagentLabel(step);
  return (
    <li data-subagent={step.id} data-status={step.status} className="min-w-0">
      <button
        type="button"
        aria-expanded={open}
        onClick={() => setChosen(!open)}
        className="flex w-full items-start gap-2 rounded-md px-1 py-0.5 text-left outline-none hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring"
      >
        <Chevron className="mt-0.5 size-3.5 shrink-0" />
        <span aria-hidden="true" className="mt-px block h-5 w-4 shrink-0">
          <PaperFigure tintClass={step.status === "failed" ? "text-red-600" : tint} />
        </span>
        <span className="font-medium text-foreground">{label.type}</span>
        <span className="min-w-0 flex-1 truncate">{label.task}</span>
        <span
          data-subagent-status=""
          className={cn("shrink-0 text-xs", step.status === "failed" && "text-red-600")}
        >
          {groupStatusLabel(step.status)}
        </span>
      </button>
      {open && (
        <div className="mt-1 ml-2 border-l border-border pl-3">
          {node.children.length > 0 && <StepRows nodes={node.children} tint={tint} />}
          <StepDetail step={step} />
        </div>
      )}
    </li>
  );
}

function Steps({ steps, tint }: { steps: Step[]; tint: string }) {
  const [open, setOpen] = useState(false);
  const Chevron = open ? ChevronDown : ChevronRight;
  return (
    <div className="my-2 text-sm text-muted-foreground">
      <button
        type="button"
        aria-expanded={open}
        onClick={() => setOpen(!open)}
        className="flex items-center gap-1 rounded-md px-1 py-0.5 outline-none hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring"
      >
        <Chevron className="size-4" />
        {steps.length === 1 ? "1 step" : `${steps.length} steps`}
      </button>
      {open && (
        <div className="pl-5">
          <StepRows nodes={buildTree(steps)} tint={tint} />
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

export function TurnView({ turn, members }: { turn: Turn; members?: GroupMember[] }) {
  const notices = turn.items.filter((item): item is TextItem => item.type === "notice");
  const steps = turn.items.filter((item): item is Step => item.type === "step");
  const texts = turn.items.filter((item): item is TextItem => item.type === "text");
  const author = members
    ? (members.find((member) => member.agentId === turn.agentId) ?? FORMER_MEMBER)
    : null;
  const reply = (
    <>
      {steps.length > 0 && <Steps steps={steps} tint={tintClass(author?.color ?? "slate")} />}
      {texts.map((item) => (
        <Markdown key={item.id} text={item.text} members={members} />
      ))}
      <HandOffMarkers turn={turn} members={members} />
      {turn.status === "running" && texts.length === 0 && steps.length === 0 && (
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
    <article className="flex flex-col gap-2">
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
        <div data-figure-scope="" className="flex gap-3">
          <span data-figure-anchor="" className="shrink-0">
            <AgentAvatar name={author.name} color={author.color} className="size-8" />
          </span>
          <div className="flex min-w-0 flex-1 flex-col gap-2">
            <div className="flex items-center gap-2">
              <span className="text-sm font-medium">{author.name}</span>
              <FigureTray steps={steps} color={author.color} />
            </div>
            {reply}
          </div>
        </div>
      ) : (
        reply
      )}
    </article>
  );
}
