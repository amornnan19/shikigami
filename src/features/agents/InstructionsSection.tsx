import { useMutation, useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { Button } from "@/components/ui/button";
import { asAgentError, readInstructions, revealWorkspace, writeInstructions } from "./api";

const CONFLICT_CODE = "changed-on-disk";
const CONFLICT_MESSAGE =
  "CLAUDE.md changed on disk since you opened it. Save again to overwrite it.";

function InstructionsEditor({
  agentId,
  initialText,
  initialModifiedAt,
}: {
  agentId: string;
  initialText: string;
  initialModifiedAt: number | null;
}) {
  const [text, setText] = useState(initialText);
  const [saved, setSaved] = useState(initialText);
  const [modifiedAt, setModifiedAt] = useState(initialModifiedAt);
  const [conflict, setConflict] = useState(false);
  const save = useMutation({
    mutationFn: () => writeInstructions(agentId, text, modifiedAt, conflict),
    onSuccess: (written) => {
      setModifiedAt(written.modifiedAt);
      setSaved(text);
      setConflict(false);
    },
    onError: (error) => {
      if (asAgentError(error).code === CONFLICT_CODE) setConflict(true);
    },
  });
  const reveal = useMutation({ mutationFn: () => revealWorkspace(agentId) });
  const failure = save.isError && !conflict ? asAgentError(save.error).message : null;
  const dirty = text !== saved;

  return (
    <div className="flex flex-col gap-2">
      <textarea
        aria-label="Instructions"
        value={text}
        spellCheck={false}
        rows={10}
        onChange={(event) => setText(event.target.value)}
        className="w-full resize-y rounded-md border border-border bg-background px-3 py-2 font-mono text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring"
      />
      {conflict && (
        <p role="alert" id="instructions-conflict" className="text-sm text-amber-700">
          {CONFLICT_MESSAGE}
        </p>
      )}
      {failure && (
        <p role="alert" className="text-sm text-red-600">
          {failure}
        </p>
      )}
      {reveal.isError && (
        <p role="alert" className="text-sm text-red-600">
          {asAgentError(reveal.error).message}
        </p>
      )}
      {save.isSuccess && !dirty && !conflict && (
        <p role="status" className="text-sm text-muted-foreground">
          Saved.
        </p>
      )}
      <div className="flex gap-2">
        <Button
          type="button"
          size="sm"
          disabled={save.isPending || (!dirty && !conflict)}
          onClick={() => save.mutate()}
        >
          Save instructions
        </Button>
        <Button type="button" size="sm" variant="outline" onClick={() => reveal.mutate()}>
          Show in Finder
        </Button>
      </div>
    </div>
  );
}

export function InstructionsSection({ agentId }: { agentId: string }) {
  const instructions = useQuery({
    queryKey: ["instructions", agentId],
    queryFn: () => readInstructions(agentId),
    retry: false,
    staleTime: 0,
    gcTime: 0,
    refetchOnMount: "always",
    refetchOnWindowFocus: false,
    refetchOnReconnect: false,
  });
  return (
    <section aria-label="Instructions" className="flex flex-col gap-3">
      <h3 className="text-sm font-semibold">Instructions</h3>
      <p className="text-sm text-muted-foreground">
        The agent reads this file, CLAUDE.md, from its folder.
      </p>
      {instructions.isPending && <p className="text-sm text-muted-foreground">Loading...</p>}
      {instructions.isError && (
        <p role="alert" className="text-sm text-red-600">
          {asAgentError(instructions.error).message}
        </p>
      )}
      {instructions.data && (
        <InstructionsEditor
          agentId={agentId}
          initialText={instructions.data.text}
          initialModifiedAt={instructions.data.modifiedAt}
        />
      )}
    </section>
  );
}
