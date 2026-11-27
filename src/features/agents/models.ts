export const CLAUDE_CODE = "claude-code";

export const MODEL_SUGGESTIONS = ["default", "opus", "sonnet", "haiku"];

export const NAME_MAX = 40;
export const MODEL_MAX = 100;

export function nameProblem(name: string): string | null {
  const length = Array.from(name.trim()).length;
  if (length === 0) return "The name cannot be empty.";
  if (length > NAME_MAX) return `Use 1 to ${NAME_MAX} characters.`;
  if (/\p{Cc}/u.test(name.trim())) {
    return "The name cannot contain line breaks or control characters.";
  }
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
