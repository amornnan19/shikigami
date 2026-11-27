import { invoke } from "@tauri-apps/api/core";

export type AgentState = "idle" | "working" | "needs-attention";

export interface Agent {
  id: string;
  name: string;
  color: string;
  provider: string;
  model: string;
  workspacePath: string;
  createdAt: number;
  lastActivityAt: number;
  state: AgentState;
}

export interface CreateAgentInput {
  name: string;
  color: string;
  provider: string;
  model: string;
}

export interface AgentError {
  code: string;
  message: string;
}

export function asAgentError(error: unknown): AgentError {
  if (
    typeof error === "object" &&
    error !== null &&
    typeof (error as AgentError).code === "string" &&
    typeof (error as AgentError).message === "string"
  ) {
    return error as AgentError;
  }
  return { code: "unknown", message: String(error) };
}

export function listAgents() {
  return invoke<Agent[]>("list_agents");
}

export function createAgent(input: CreateAgentInput) {
  return invoke<Agent>("create_agent", { input });
}
