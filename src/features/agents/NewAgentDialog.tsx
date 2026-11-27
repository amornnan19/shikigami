import * as RadioGroup from "@radix-ui/react-radio-group";
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
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { cn } from "@/lib/utils";
import { type Agent, type AgentError, asAgentError, createAgent } from "./api";
import { AGENT_COLORS } from "./colors";
import { CLAUDE_CODE, MODEL_SUGGESTIONS, modelProblem, nameProblem } from "./models";

const OTHER_MODEL = "__other__";
const DEFAULT_MODEL = "default";
const NAME_CODES = ["name-empty", "name-too-long", "name-taken"];

function FieldError({ id, message }: { id: string; message: string | null }) {
  if (!message) return null;
  return (
    <p id={id} role="alert" className="text-sm text-red-600">
      {message}
    </p>
  );
}

function errorFor(error: AgentError | null, codes: string[]) {
  return error && codes.includes(error.code) ? error.message : null;
}

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
  const [modelChoice, setModelChoice] = useState(DEFAULT_MODEL);
  const [customModel, setCustomModel] = useState("");
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

  const model = modelChoice === OTHER_MODEL ? customModel.trim() : modelChoice;

  const clientNameProblem = nameProblem(name);
  const clientModelProblem = modelChoice === OTHER_MODEL ? modelProblem(customModel) : null;
  const valid = clientNameProblem === null && clientModelProblem === null && !create.isPending;

  const nameError = errorFor(serverError, NAME_CODES);
  const modelError =
    errorFor(serverError, ["model-invalid"]) ?? (customModel === "" ? null : clientModelProblem);
  const colorError = errorFor(serverError, ["color-invalid"]);
  const formError =
    serverError && ![...NAME_CODES, "model-invalid", "color-invalid"].includes(serverError.code)
      ? serverError.message
      : null;

  function submit(event: FormEvent) {
    event.preventDefault();
    if (!valid) return;
    setServerError(null);
    create.mutate({ name, color, provider: CLAUDE_CODE, model });
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

      <div className="flex flex-col gap-2">
        <Label id="agent-color-label">Colour</Label>
        <RadioGroup.Root
          aria-labelledby="agent-color-label"
          value={color}
          onValueChange={setColor}
          className="flex flex-wrap gap-2"
        >
          {AGENT_COLORS.map((entry) => (
            <RadioGroup.Item
              key={entry.name}
              value={entry.name}
              aria-label={entry.label}
              className={cn(
                "size-7 rounded-full outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 data-[state=checked]:ring-2 data-[state=checked]:ring-foreground data-[state=checked]:ring-offset-2",
                entry.swatch,
              )}
            />
          ))}
        </RadioGroup.Root>
        <FieldError id="agent-color-error" message={colorError} />
      </div>

      <div className="flex flex-col gap-2">
        <Label htmlFor="agent-model">Model</Label>
        <Select
          value={modelChoice}
          onValueChange={(value) => {
            setModelChoice(value);
            setServerError(null);
          }}
        >
          <SelectTrigger id="agent-model" aria-describedby="agent-model-error">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {MODEL_SUGGESTIONS.map((suggestion) => (
              <SelectItem key={suggestion} value={suggestion}>
                {suggestion}
              </SelectItem>
            ))}
            <SelectItem value={OTHER_MODEL}>Other...</SelectItem>
          </SelectContent>
        </Select>
        {modelChoice === OTHER_MODEL && (
          <Input
            aria-label="Model name"
            value={customModel}
            autoComplete="off"
            aria-invalid={modelError !== null}
            aria-describedby="agent-model-error"
            onChange={(event) => {
              setCustomModel(event.target.value);
              setServerError(null);
            }}
          />
        )}
        <FieldError id="agent-model-error" message={modelError} />
      </div>

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
