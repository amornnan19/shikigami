import { useMutation, useQueryClient } from "@tanstack/react-query";
import { type FormEvent, useState } from "react";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  APP_DEFAULT_AUTH,
  authModeOf,
  ColorField,
  DEFAULT_MODEL,
  errorFor,
  FieldError,
  ModelField,
  NAME_CODES,
  SignInField,
  useModelChoice,
} from "./AgentFields";
import { type Agent, type AgentError, asAgentError, createAgent } from "./api";
import { AGENT_COLORS } from "./colors";
import { CLAUDE_CODE, nameProblem } from "./models";

function NewAgentForm({
  onOpenChange,
  onCreated,
}: {
  onOpenChange: (open: boolean) => void;
  onCreated: (agent: Agent) => void;
}) {
  const queryClient = useQueryClient();
  const [name, setName] = useState("");
  const [color, setColor] = useState<string>(AGENT_COLORS[0].name);
  const modelState = useModelChoice(DEFAULT_MODEL);
  const [authChoice, setAuthChoice] = useState<string>(APP_DEFAULT_AUTH);
  const [serverError, setServerError] = useState<AgentError | null>(null);

  const create = useMutation({
    mutationFn: createAgent,
    onSuccess: async (agent) => {
      await queryClient.invalidateQueries({ queryKey: ["agents"] });
      onCreated(agent);
      onOpenChange(false);
    },
    onError: (error) => setServerError(asAgentError(error)),
  });

  const clientNameProblem = nameProblem(name);
  const valid = clientNameProblem === null && modelState.problem === null && !create.isPending;

  const nameError = errorFor(serverError, NAME_CODES);
  const modelError =
    errorFor(serverError, ["model-invalid"]) ??
    (modelState.custom === "" ? null : modelState.problem);
  const colorError = errorFor(serverError, ["color-invalid"]);
  const formError =
    serverError && ![...NAME_CODES, "model-invalid", "color-invalid"].includes(serverError.code)
      ? serverError.message
      : null;

  function submit(event: FormEvent) {
    event.preventDefault();
    if (!valid) return;
    setServerError(null);
    create.mutate({
      name,
      color,
      provider: CLAUDE_CODE,
      model: modelState.model,
      authMode: authModeOf(authChoice),
    });
  }

  return (
    <form className="flex flex-col gap-4" onSubmit={submit}>
      <div className="flex flex-col gap-2">
        <Label htmlFor="agent-name">Name</Label>
        <Input
          id="agent-name"
          value={name}
          autoComplete="off"
          aria-invalid={nameError !== null}
          aria-describedby="agent-name-error"
          onChange={(event) => {
            setName(event.target.value);
            setServerError(null);
          }}
        />
        <FieldError id="agent-name-error" message={nameError} />
      </div>

      <ColorField idPrefix="agent" value={color} onChange={setColor} error={colorError} />

      <ModelField
        idPrefix="agent"
        state={modelState}
        error={modelError}
        onEdit={() => setServerError(null)}
      />

      <SignInField id="agent-sign-in" value={authChoice} onChange={setAuthChoice} />

      <FieldError id="agent-form-error" message={formError} />
      <DialogFooter>
        <Button type="button" variant="ghost" onClick={() => onOpenChange(false)}>
          Cancel
        </Button>
        <Button type="submit" disabled={!valid}>
          Create
        </Button>
      </DialogFooter>
    </form>
  );
}

export function NewAgentDialog({
  open,
  onOpenChange,
  onCreated,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onCreated: (agent: Agent) => void;
}) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent aria-describedby="new-agent-description">
        <DialogHeader>
          <DialogTitle>New agent</DialogTitle>
          <DialogDescription id="new-agent-description">
            An agent gets its own folder and keeps its chat.
          </DialogDescription>
        </DialogHeader>
        <NewAgentForm onOpenChange={onOpenChange} onCreated={onCreated} />
      </DialogContent>
    </Dialog>
  );
}
