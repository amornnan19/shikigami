import * as RadioGroup from "@radix-ui/react-radio-group";
import { useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  AUTH_MODE_LABELS,
  type AuthMode,
  CLAUDE_AUTH_KEY,
  getClaudeAuth,
} from "@/features/providers/api";
import { cn } from "@/lib/utils";
import type { AgentError } from "./api";
import { AGENT_COLORS } from "./colors";
import { MODEL_SUGGESTIONS, modelProblem } from "./models";

const OTHER_MODEL = "__other__";
export const DEFAULT_MODEL = "default";
export const APP_DEFAULT_AUTH = "app-default";
export const NAME_CODES = ["name-empty", "name-too-long", "name-taken"];

export function FieldError({ id, message }: { id: string; message: string | null }) {
  if (!message) return null;
  return (
    <p id={id} role="alert" className="text-sm text-red-600">
      {message}
    </p>
  );
}

export function errorFor(error: AgentError | null, codes: string[]) {
  return error && codes.includes(error.code) ? error.message : null;
}

export function authChoiceOf(mode: AuthMode | null) {
  return mode ?? APP_DEFAULT_AUTH;
}

export function authModeOf(choice: string): AuthMode | null {
  return choice === APP_DEFAULT_AUTH ? null : (choice as AuthMode);
}

export function ColorField({
  idPrefix,
  value,
  onChange,
  error,
}: {
  idPrefix: string;
  value: string;
  onChange: (color: string) => void;
  error: string | null;
}) {
  return (
    <div className="flex flex-col gap-2">
      <Label id={`${idPrefix}-color-label`}>Colour</Label>
      <RadioGroup.Root
        aria-labelledby={`${idPrefix}-color-label`}
        value={value}
        onValueChange={onChange}
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
      <FieldError id={`${idPrefix}-color-error`} message={error} />
    </div>
  );
}

export function useModelChoice(initial: string) {
  const known = MODEL_SUGGESTIONS.includes(initial);
  const [choice, setChoice] = useState(known ? initial : OTHER_MODEL);
  const [custom, setCustom] = useState(known ? "" : initial);
  const other = choice === OTHER_MODEL;
  return {
    choice,
    custom,
    setChoice,
    setCustom,
    other,
    model: other ? custom.trim() : choice,
    problem: other ? modelProblem(custom) : null,
  };
}

export type ModelChoice = ReturnType<typeof useModelChoice>;

export function ModelField({
  idPrefix,
  state,
  error,
  onEdit,
}: {
  idPrefix: string;
  state: ModelChoice;
  error: string | null;
  onEdit: () => void;
}) {
  return (
    <div className="flex flex-col gap-2">
      <Label htmlFor={`${idPrefix}-model`}>Model</Label>
      <Select
        value={state.choice}
        onValueChange={(value) => {
          state.setChoice(value);
          onEdit();
        }}
      >
        <SelectTrigger id={`${idPrefix}-model`} aria-describedby={`${idPrefix}-model-error`}>
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
      {state.other && (
        <Input
          aria-label="Model name"
          value={state.custom}
          autoComplete="off"
          aria-invalid={error !== null}
          aria-describedby={`${idPrefix}-model-error`}
          onChange={(event) => {
            state.setCustom(event.target.value);
            onEdit();
          }}
        />
      )}
      <FieldError id={`${idPrefix}-model-error`} message={error} />
    </div>
  );
}

export function SignInField({
  id,
  value,
  onChange,
}: {
  id: string;
  value: string;
  onChange: (choice: string) => void;
}) {
  const claudeAuth = useQuery({ queryKey: CLAUDE_AUTH_KEY, queryFn: getClaudeAuth, retry: false });
  const appDefaultLabel = claudeAuth.data
    ? `Use app default (${AUTH_MODE_LABELS[claudeAuth.data.appMode]})`
    : "Use app default";
  return (
    <div className="flex flex-col gap-2">
      <Label htmlFor={id}>Sign-in</Label>
      <Select value={value} onValueChange={onChange}>
        <SelectTrigger id={id}>
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value={APP_DEFAULT_AUTH}>{appDefaultLabel}</SelectItem>
          <SelectItem value="subscription">Claude subscription</SelectItem>
          <SelectItem value="api-key">API key</SelectItem>
        </SelectContent>
      </Select>
    </div>
  );
}
