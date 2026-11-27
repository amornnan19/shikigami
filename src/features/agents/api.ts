import { invoke } from "@tauri-apps/api/core";
import type { AuthMode } from "@/features/providers/api";

export type AgentState = "idle" | "working" | "needs-attention";

export interface Attention {
  code: string;
  message: string;
}

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
  attention: Attention | null;
  authMode: AuthMode | null;
  effectiveAuthMode: AuthMode;
}

export interface CreateAgentInput {
  name: string;
  color: string;
  provider: string;
  model: string;
  authMode: AuthMode | null;
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

export function recreateWorkspace(agentId: string) {
  return invoke<Agent>("recreate_workspace", { agentId });
}

export interface UpdateAgentInput {
  name?: string;
  color?: string;
  model?: string;
}

export interface Instructions {
  text: string;
  modifiedAt: number | null;
}

export interface Written {
  modifiedAt: number | null;
}

export interface Deleted {
  folderKept: boolean;
  reason: string | null;
}

export function updateAgent(agentId: string, input: UpdateAgentInput) {
  return invoke<Agent>("update_agent", { agentId, input });
}

export function setAgentAuthMode(agentId: string, mode: AuthMode | null) {
  return invoke<Agent>("set_agent_auth_mode", { agentId, mode });
}

export function readInstructions(agentId: string) {
  return invoke<Instructions>("read_instructions", { agentId });
}

export function writeInstructions(
  agentId: string,
  text: string,
  expectedModifiedAt: number | null,
  force: boolean,
) {
  return invoke<Written>("write_instructions", { agentId, text, expectedModifiedAt, force });
}

export function revealWorkspace(agentId: string) {
  return invoke<void>("reveal_workspace", { agentId });
}

export function deleteAgent(agentId: string, moveFolderToTrash: boolean) {
  return invoke<Deleted>("delete_agent", { agentId, moveFolderToTrash });
}
