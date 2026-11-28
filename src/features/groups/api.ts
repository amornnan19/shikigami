import { invoke } from "@tauri-apps/api/core";
import type { Attention } from "@/features/agents/api";

export interface GroupMember {
  agentId: string;
  name: string;
  color: string;
}

export interface Group {
  id: string;
  name: string;
  folderPath: string;
  members: GroupMember[];
  leadAgentId: string | null;
  createdAt: number;
  lastActivityAt: number;
  attention: Attention | null;
  waitingForYou: boolean;
}

export interface CreateGroupInput {
  name: string;
  folderPath: string | null;
  memberIds: string[];
  leadAgentId: string;
}

export interface UpdateGroupInput {
  name?: string;
  memberIds?: string[];
  leadAgentId?: string;
}

export interface CreatedGroup {
  group: Group;
  sharedFolderWith: string[];
}

export const GROUPS_KEY = ["groups"];

export const SHARED_FOLDER_NOTICE = "Another group uses this folder.";

export function listGroups() {
  return invoke<Group[]>("list_groups");
}

export function createGroup(input: CreateGroupInput) {
  return invoke<CreatedGroup>("create_group", { input });
}

export function updateGroup(groupId: string, input: UpdateGroupInput) {
  return invoke<Group>("update_group", { groupId, input });
}

export function deleteGroup(groupId: string) {
  return invoke<void>("delete_group", { groupId });
}

export function pickGroupFolder() {
  return invoke<string | null>("pick_group_folder");
}

export function suggestGroupFolder(name: string) {
  return invoke<string>("suggest_group_folder", { name });
}
