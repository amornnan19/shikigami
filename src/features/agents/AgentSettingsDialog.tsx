import { useMutation, useQueryClient } from "@tanstack/react-query";
import { type ReactNode, useState } from "react";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  authChoiceOf,
  authModeOf,
  ColorField,
  errorFor,
  FieldError,
  ModelField,
  NAME_CODES,
  SignInField,
  useModelChoice,
} from "./AgentFields";
import {
  type Agent,
  type AgentError,
  asAgentError,
  type Deleted,
  setAgentAuthMode,
  updateAgent,
} from "./api";
import { DeleteAgentDialog } from "./DeleteAgentDialog";
import { InstructionsSection } from "./InstructionsSection";
import { nameProblem } from "./models";

function Section({ label, children }: { label: string; children: ReactNode }) {
  return (
    <section aria-label={label} className="flex flex-col gap-3 border-t border-border pt-4">
      <h3 className="text-sm font-semibold">{label}</h3>
      {children}
    </section>
  );
}

function useAgentsRefresh() {
  const client = useQueryClient();
  return () => client.invalidateQueries({ queryKey: ["agents"] });
}

function IdentitySection({ agent }: { agent: Agent }) {
  const refresh = useAgentsRefresh();
  const [name, setName] = useState(agent.name);
  const [color, setColor] = useState(agent.color);
  const [error, setError] = useState<AgentError | null>(null);
  const save = useMutation({
    mutationFn: () => updateAgent(agent.id, { name, color }),
    onSuccess: refresh,
    onError: (failure) => setError(asAgentError(failure)),
  });
  const changed = name.trim() !== agent.name || color !== agent.color;
  const valid = nameProblem(name) === null && changed && !save.isPending;
  const nameError = errorFor(error, NAME_CODES);
  const colorError = errorFor(error, ["color-invalid"]);
  const otherError = error && !nameError && !colorError ? error.message : null;

  return (
    <Section label="Identity">
      <div className="flex flex-col gap-2">
        <Label htmlFor="settings-name">Name</Label>
        <Input
          id="settings-name"
          value={name}
          autoComplete="off"
          aria-invalid={nameError !== null}
          aria-describedby="settings-name-error"
          onChange={(event) => {
            setName(event.target.value);
            setError(null);
          }}
        />
        <FieldError id="settings-name-error" message={nameError} />
      </div>
      <ColorField
        idPrefix="settings"
        value={color}
        onChange={(next) => {
          setColor(next);
          setError(null);
        }}
        error={colorError}
      />
      <FieldError id="settings-identity-error" message={otherError} />
      <div>
        <Button type="button" size="sm" disabled={!valid} onClick={() => save.mutate()}>
          Save identity
        </Button>
      </div>
    </Section>
  );
}

function ModelSection({ agent }: { agent: Agent }) {
  const refresh = useAgentsRefresh();
  const modelState = useModelChoice(agent.model);
  const [error, setError] = useState<AgentError | null>(null);
  const save = useMutation({
    mutationFn: () => updateAgent(agent.id, { model: modelState.model }),
    onSuccess: refresh,
    onError: (failure) => setError(asAgentError(failure)),
  });
  const valid = modelState.problem === null && modelState.model !== agent.model && !save.isPending;
  const modelError =
    errorFor(error, ["model-invalid"]) ?? (modelState.custom === "" ? null : modelState.problem);
  const otherError = error && error.code !== "model-invalid" ? error.message : null;

  return (
    <Section label="Model">
      <ModelField
        idPrefix="settings"
        state={modelState}
        error={modelError}
        onEdit={() => setError(null)}
      />
      <p className="text-sm text-muted-foreground">A new model applies from the next message.</p>
      <FieldError id="settings-model-form-error" message={otherError} />
      <div>
        <Button type="button" size="sm" disabled={!valid} onClick={() => save.mutate()}>
          Save model
        </Button>
      </div>
    </Section>
  );
}

function SignInSection({ agent }: { agent: Agent }) {
  const refresh = useAgentsRefresh();
  const [choice, setChoice] = useState(authChoiceOf(agent.authMode));
  const save = useMutation({
    mutationFn: () => setAgentAuthMode(agent.id, authModeOf(choice)),
    onSuccess: refresh,
  });
  const changed = choice !== authChoiceOf(agent.authMode);

  return (
    <Section label="Sign-in">
      <SignInField
        id="settings-sign-in"
        value={choice}
        onChange={(next) => {
          setChoice(next);
          save.reset();
        }}
      />
      {save.isError && (
        <p role="alert" className="text-sm text-red-600">
          {asAgentError(save.error).message}
        </p>
      )}
      <div>
        <Button
          type="button"
          size="sm"
          disabled={!changed || save.isPending}
          onClick={() => save.mutate()}
        >
          Save sign-in
        </Button>
      </div>
    </Section>
  );
}

function DeleteSection({
  agent,
  running,
  onDeleted,
}: {
  agent: Agent;
  running: boolean;
  onDeleted: (outcome: Deleted, askedForTrash: boolean) => void;
}) {
  const [confirming, setConfirming] = useState(false);
  return (
    <Section label="Delete">
      <div className="flex items-center gap-3">
        <Button
          type="button"
          size="sm"
          variant="destructive"
          disabled={running}
          onClick={() => setConfirming(true)}
        >
          Delete agent
        </Button>
        {running && (
          <span id="delete-hint" className="text-sm text-muted-foreground">
            Stop the agent before deleting it.
          </span>
        )}
      </div>
      <DeleteAgentDialog
        agent={agent}
        open={confirming}
        onOpenChange={setConfirming}
        onDeleted={onDeleted}
      />
    </Section>
  );
}

export function AgentSettingsDialog({
  agent,
  running,
  open,
  onOpenChange,
  onDeleted,
}: {
  agent: Agent;
  running: boolean;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onDeleted: (outcome: Deleted, askedForTrash: boolean) => void;
}) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent
        aria-describedby="agent-settings-description"
        className="max-h-[90vh] max-w-2xl overflow-y-auto"
      >
        <DialogHeader>
          <DialogTitle>Agent settings</DialogTitle>
          <DialogDescription id="agent-settings-description">
            Changes keep the conversation and the folder.
          </DialogDescription>
        </DialogHeader>
        <IdentitySection agent={agent} />
        <ModelSection agent={agent} />
        <SignInSection agent={agent} />
        <InstructionsSection agentId={agent.id} />
        <DeleteSection agent={agent} running={running} onDeleted={onDeleted} />
      </DialogContent>
    </Dialog>
  );
}
