import {
  type Conversation,
  editGroupQueued,
  editQueued,
  getConversation,
  getGroupConversation,
  listGroupQueue,
  listQueue,
  type QueuedMessage,
  removeGroupQueued,
  removeQueued,
  resumeGroupQueue,
  resumeQueue,
  type SendResult,
  sendGroupMessage,
  sendMessage,
  stopGroupTurn,
  stopTurn,
} from "./api";

type QueryKey = readonly unknown[];

export interface ChatScope {
  id: string;
  conversationKey: QueryKey;
  queueKey: QueryKey;
  refreshKeys: QueryKey[];
  load: (beforeTurnId?: string) => Promise<Conversation>;
  loadQueue: () => Promise<QueuedMessage[]>;
  send: (text: string) => Promise<SendResult>;
  stop: (turnId: string) => Promise<void>;
  resume: () => Promise<void>;
  editQueued: (queuedId: string, text: string) => Promise<QueuedMessage>;
  removeQueued: (queuedId: string) => Promise<void>;
}

export const agentConversationKey = (agentId: string): QueryKey => ["conversation", agentId];
export const agentQueueKey = (agentId: string): QueryKey => ["queue", agentId];
export const groupConversationKey = (groupId: string): QueryKey => ["group-conversation", groupId];
export const groupQueueKey = (groupId: string): QueryKey => ["group-queue", groupId];

export function agentScope(agentId: string): ChatScope {
  return {
    id: agentId,
    conversationKey: agentConversationKey(agentId),
    queueKey: agentQueueKey(agentId),
    refreshKeys: [["agents"]],
    load: (beforeTurnId) => getConversation(agentId, beforeTurnId),
    loadQueue: () => listQueue(agentId),
    send: (text) => sendMessage(agentId, text),
    stop: (turnId) => stopTurn(agentId, turnId),
    resume: () => resumeQueue(agentId),
    editQueued,
    removeQueued,
  };
}

export function groupScope(groupId: string): ChatScope {
  return {
    id: groupId,
    conversationKey: groupConversationKey(groupId),
    queueKey: groupQueueKey(groupId),
    refreshKeys: [["agents"], ["groups"]],
    load: (beforeTurnId) => getGroupConversation(groupId, beforeTurnId),
    loadQueue: () => listGroupQueue(groupId),
    send: (text) => sendGroupMessage(groupId, text),
    stop: (turnId) => stopGroupTurn(groupId, turnId),
    resume: () => resumeGroupQueue(groupId),
    editQueued: editGroupQueued,
    removeQueued: removeGroupQueued,
  };
}
