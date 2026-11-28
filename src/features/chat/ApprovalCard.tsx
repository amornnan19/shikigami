import { useMutation } from "@tanstack/react-query";
import { Check, ShieldQuestion, X } from "lucide-react";
import { useState } from "react";
import { Button } from "@/components/ui/button";
import { AgentAvatar } from "@/features/agents/AgentAvatar";
import { cn } from "@/lib/utils";
import { type Approval, answerApproval } from "./api";

const OUTCOME_LABEL = {
  allowed: "Allowed",
  "timed-out": "Timed out",
} as const;

export function outcomeLabel(approval: Approval): string {
  if (approval.status === "pending") return "Waiting for you";
  if (approval.status === "allowed" || approval.status === "timed-out") {
    return OUTCOME_LABEL[approval.status];
  }
  if (approval.answeredBy === "stop") return "Denied because the turn stopped";
  if (approval.answeredBy === "relay") return "Denied because the request was interrupted";
  if (approval.answeredBy === "shutdown") return "Denied because Shikigami closed";
  return "Denied";
}

export function folderName(path: string): string {
  const parts = path.split("/").filter(Boolean);
  return parts[parts.length - 1] ?? path;
}

const HIDDEN = /[\p{Cc}؜​-‏‪-‮⁠-⁩﻿]/gu;

export function visibleText(text: string): string {
  return text.replace(HIDDEN, (character) => {
    if (character === "\n" || character === "\t") return character;
    const code = (character.codePointAt(0) ?? 0).toString(16).toUpperCase().padStart(4, "0");
    return `\\u${code}`;
  });
}

function timeOf(millis: number): string {
  return new Date(millis).toLocaleTimeString("en-US", { hour: "numeric", minute: "2-digit" });
}

export function ApprovalCard({
  approval,
  name,
  color,
  showName = true,
}: {
  approval: Approval;
  name: string;
  color: string;
  showName?: boolean;
}) {
  const [expanded, setExpanded] = useState(false);
  const answer = useMutation({
    mutationFn: (allow: boolean) => answerApproval(approval.id, allow),
  });
  const pending = approval.status === "pending";
  const verb = approval.tool === "Bash" ? "wants to run" : "wants to use";
  const folder = folderName(approval.folder);
  return (
    <section
      aria-label="Approval request"
      data-approval={approval.id}
      data-status={approval.status}
      className={cn(
        "flex flex-col gap-2 rounded-lg border p-3 text-sm",
        pending ? "border-amber-500/50 bg-amber-500/5" : "border-border bg-muted/30",
      )}
    >
      <div className="flex items-center gap-2">
        <AgentAvatar name={name} color={color} className="size-6 text-xs" />
        {showName && <span className="font-medium">{name}</span>}
        <span className="text-muted-foreground">{verb}</span>
      </div>
      <pre
        data-approval-summary=""
        className="max-h-60 overflow-auto rounded-md border border-border bg-muted p-2 font-mono text-xs whitespace-pre-wrap break-all"
      >
        {visibleText(approval.summary)}
      </pre>
      <p className="text-xs text-muted-foreground">
        in{" "}
        <span
          data-approval-where=""
          title={approval.folder}
          className="font-mono break-all text-foreground"
        >
          {visibleText(approval.target ?? folder)}
        </span>
      </p>
      {approval.tool === "Bash" && (
        <p className="text-xs text-muted-foreground">
          Runs in the sandbox: no network, writes only in {folder}.
        </p>
      )}
      {approval.reason && (
        <p data-approval-reason="" className="text-xs break-words text-muted-foreground italic">
          {name} says: {visibleText(approval.reason)}
        </p>
      )}
      <div>
        <button
          type="button"
          aria-expanded={expanded}
          onClick={() => setExpanded(!expanded)}
          className="rounded-md text-xs underline outline-none focus-visible:ring-2 focus-visible:ring-ring"
        >
          {expanded ? "Hide full request" : "Show full request"}
        </button>
        {expanded && (
          <pre
            data-approval-detail=""
            className="mt-1 max-h-72 overflow-auto rounded-md border border-border bg-muted p-2 font-mono text-xs whitespace-pre-wrap break-all"
          >
            {visibleText(approval.detail)}
          </pre>
        )}
      </div>
      {pending ? (
        <div className="flex items-center gap-2">
          <Button size="sm" disabled={answer.isPending} onClick={() => answer.mutate(true)}>
            Allow once
          </Button>
          <Button
            size="sm"
            variant="outline"
            disabled={answer.isPending}
            onClick={() => answer.mutate(false)}
          >
            Deny
          </Button>
          {answer.isError && (
            <span role="alert" className="text-xs text-red-600">
              This request can no longer be answered.
            </span>
          )}
        </div>
      ) : (
        <p data-approval-outcome="" className="flex items-center gap-1.5 text-xs">
          {approval.status === "allowed" ? (
            <Check className="size-3.5 text-green-600" />
          ) : approval.status === "timed-out" ? (
            <ShieldQuestion className="size-3.5 text-muted-foreground" />
          ) : (
            <X className="size-3.5 text-red-600" />
          )}
          <span className="font-medium">{outcomeLabel(approval)}</span>
          {approval.answeredAt !== null && (
            <span className="text-muted-foreground">{timeOf(approval.answeredAt)}</span>
          )}
        </p>
      )}
    </section>
  );
}
