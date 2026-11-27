import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export const MESSAGE_MAX_CHARS = 100_000;

export type TurnStatus = "running" | "completed" | "stopped" | "failed" | "interrupted";
export type StepStatus = "running" | "done" | "stopped" | "failed";

export interface Step {
  type: "step";
  id: string;
  kind: string;
  title: string;
  detail: string | null;
  status: StepStatus;
}

export interface TextItem {
  type: "text" | "notice";
  id: string;
  text: string;
}

export type Item = Step | TextItem;

export interface Turn {
  id: string;
  status: TurnStatus;
  error: string | null;
  startedAt: number;
  endedAt: number | null;
  userMessage: string;
  items: Item[];
}

export interface Conversation {
  turns: Turn[];
  hasMore: boolean;
}

export type TurnEvent = { agentId: string; turnId: string } & (
  | { kind: "started" }
  | { kind: "text-delta"; itemId: string; text: string }
  | { kind: "text-set"; itemId: string; text: string }
  | { kind: "step-started"; step: Omit<Step, "type"> }
  | { kind: "step-ended"; stepId: string; status: StepStatus; detail: string | null }
  | { kind: "notice"; itemId: string; text: string }
  | { kind: "ended"; status: TurnStatus; error: string | null }
);

export function getConversation(agentId: string, beforeTurnId?: string) {
  return invoke<Conversation>("get_conversation", { agentId, beforeTurnId });
}

export function sendMessage(agentId: string, text: string) {
  return invoke<{ turnId: string }>("send_message", { agentId, text });
}

export function onTurnEvent(handler: (event: TurnEvent) => void) {
  return listen<TurnEvent>("turn-event", (event) => handler(event.payload));
}
