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
import type { GroupMember } from "@/features/groups/api";
import { splitReply } from "@/features/groups/mentions";
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

function Steps({ steps }: { steps: Step[] }) {
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
        <ul className="mt-1 flex flex-col gap-2 pl-5">
          {steps.map((step) => (
            <li key={step.id} className="min-w-0">
              <div className="flex items-start gap-2">
                <StepIcon status={step.status} />
                <span className="font-medium text-foreground">{step.kind}</span>
                <span className="min-w-0 truncate">{step.title}</span>
              </div>
              {step.detail && (
                <pre className="mt-1 max-h-60 overflow-auto rounded-md border border-border bg-muted p-2 font-mono text-xs whitespace-pre-wrap break-all">
                  {step.detail}
                </pre>
              )}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

const FORMER_MEMBER: Pick<GroupMember, "name" | "color"> = {
  name: "Former member",
  color: "slate",
};

function Reply({ text, turn, members }: { text: string; turn: Turn; members?: GroupMember[] }) {
  if (!members) return <Markdown text={text} />;
  return (
    <>
      {splitReply(text, members, turn.agentId).map((segment, index) =>
        segment.type === "text" ? (
          // biome-ignore lint/suspicious/noArrayIndexKey: segments have no identity of their own
          <Markdown key={index} text={segment.text} members={members} />
        ) : (
          // biome-ignore lint/suspicious/noArrayIndexKey: segments have no identity of their own
          <div key={index} data-hand-off={segment.member.agentId}>
            <p className="my-1 flex items-center gap-1.5 text-xs text-muted-foreground">
              <ArrowRight className="size-3.5" />
              Handed to <MentionChip member={segment.member} />
            </p>
            <Markdown text={segment.text} members={members} />
          </div>
        ),
      )}
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
      {steps.length > 0 && <Steps steps={steps} />}
      {texts.map((item) => (
        <Reply key={item.id} text={item.text} turn={turn} members={members} />
      ))}
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
      ) : (
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
          <AgentAvatar name={author.name} color={author.color} className="size-8" />
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
