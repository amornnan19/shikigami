export const MODEL_SUGGESTIONS: Record<string, string[]> = {
  "claude-code": ["default", "opus", "sonnet", "haiku"],
  codex: ["default", "gpt-5-codex", "gpt-5"],
};

export const NAME_MAX = 40;
export const MODEL_MAX = 100;

export function nameProblem(name: string): string | null {
  const length = Array.from(name.trim()).length;
  if (length === 0) return "Enter a name.";
  if (length > NAME_MAX) return `Use at most ${NAME_MAX} characters.`;
  return null;
}

export function modelProblem(model: string): string | null {
  const trimmed = model.trim();
  const length = Array.from(trimmed).length;
  if (length === 0 || length > MODEL_MAX || /\s/u.test(trimmed)) {
    return `Use 1 to ${MODEL_MAX} characters with no spaces.`;
  }
  return null;
}
