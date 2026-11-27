import { Check, ChevronDown, ChevronRight, LoaderCircle, Square, X } from "lucide-react";
import { useState } from "react";
import { cn } from "@/lib/utils";
import type { Step, StepStatus, TextItem, Turn } from "./api";
import { Markdown } from "./Markdown";

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

export function TurnView({ turn }: { turn: Turn }) {
  const notices = turn.items.filter((item): item is TextItem => item.type === "notice");
  const steps = turn.items.filter((item): item is Step => item.type === "step");
  const texts = turn.items.filter((item): item is TextItem => item.type === "text");
  return (
    <article className="flex flex-col gap-2">
      <div className="ml-auto max-w-[80%] rounded-2xl bg-primary px-4 py-2 whitespace-pre-wrap break-words text-primary-foreground">
        {turn.userMessage}
      </div>
      {notices.map((item) => (
        <p key={item.id} className="my-1 text-center text-xs text-muted-foreground">
          {item.text}
        </p>
      ))}
      {steps.length > 0 && <Steps steps={steps} />}
      {texts.map((item) => (
        <Markdown key={item.id} text={item.text} />
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
    </article>
  );
}
