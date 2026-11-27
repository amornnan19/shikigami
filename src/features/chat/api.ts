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

export interface QueuedMessage {
  id: string;
  text: string;
  position: number;
  createdAt: number;
}

export type SendResult = { kind: "started"; turnId: string } | { kind: "queued"; queuedId: string };

export function sendMessage(agentId: string, text: string) {
  return invoke<SendResult>("send_message", { agentId, text });
}

export function stopTurn(agentId: string, turnId: string) {
  return invoke<void>("stop_turn", { agentId, turnId });
}

export function resumeQueue(agentId: string) {
  return invoke<void>("resume_queue", { agentId });
}

export function listQueue(agentId: string) {
  return invoke<QueuedMessage[]>("list_queue", { agentId });
}

export function editQueued(queuedId: string, text: string) {
  return invoke<QueuedMessage>("edit_queued", { queuedId, text });
}

export function removeQueued(queuedId: string) {
  return invoke<void>("remove_queued", { queuedId });
}

export function onQueueChanged(handler: (agentId: string) => void) {
  return listen<{ agentId: string }>("queue-changed", (event) => handler(event.payload.agentId));
}

export function onTurnEvent(handler: (event: TurnEvent) => void) {
  return listen<TurnEvent>("turn-event", (event) => handler(event.payload));
}
