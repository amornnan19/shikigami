import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback, useEffect, useState } from "react";
import {
  type Conversation,
  onGroupQueueChanged,
  onQueueChanged,
  onTurnEvent,
  type TurnEvent,
} from "./api";
import { applyEvent, mergeNewest, prependOlder } from "./conversation";
import {
  agentConversationKey,
  agentQueueKey,
  type ChatScope,
  groupConversationKey,
  groupQueueKey,
} from "./scope";

const eventKey = (event: TurnEvent) =>
  event.groupId ? groupConversationKey(event.groupId) : agentConversationKey(event.agentId);

export function useTurnEvents() {
  const client = useQueryClient();
  useEffect(() => {
    const stop = onTurnEvent((event) => {
      if (event.kind === "started" || event.kind === "ended") {
        void client.invalidateQueries({ queryKey: eventKey(event) });
        void client.invalidateQueries({ queryKey: ["agents"] });
        if (event.groupId) void client.invalidateQueries({ queryKey: ["groups"] });
      }
      if (event.kind === "approval-requested" || event.kind === "approval-resolved") {
        void client.invalidateQueries({ queryKey: ["agents"] });
        if (event.groupId) void client.invalidateQueries({ queryKey: ["groups"] });
      }
      client.setQueryData<Conversation>(eventKey(event), (data) =>
        data ? applyEvent(data, event) : data,
      );
    });
    return () => {
      void stop.then((unlisten) => unlisten());
    };
  }, [client]);
  useEffect(() => {
    const stop = onQueueChanged((agentId) => {
      void client.invalidateQueries({ queryKey: agentQueueKey(agentId) });
    });
    return () => {
      void stop.then((unlisten) => unlisten());
    };
  }, [client]);
  useEffect(() => {
    const stop = onGroupQueueChanged((groupId) => {
      void client.invalidateQueries({ queryKey: groupQueueKey(groupId) });
    });
    return () => {
      void stop.then((unlisten) => unlisten());
    };
  }, [client]);
}

export function useQueue(scope: ChatScope) {
  return useQuery({ queryKey: scope.queueKey, queryFn: scope.loadQueue, retry: false });
}

export function useConversation(scope: ChatScope) {
  const client = useQueryClient();
  const [loadingOlder, setLoadingOlder] = useState(false);
  const { conversationKey, load } = scope;
  const query = useQuery({
    queryKey: conversationKey,
    queryFn: async () => {
      const fresh = await load();
      return mergeNewest(client.getQueryData<Conversation>(conversationKey), fresh);
    },
    retry: false,
  });
  const oldestId = query.data?.turns[0]?.id;

  const loadOlder = useCallback(async () => {
    if (!oldestId) return;
    setLoadingOlder(true);
    try {
      const older = await load(oldestId);
      client.setQueryData<Conversation>(conversationKey, (data) =>
        data ? prependOlder(data, older) : data,
      );
    } finally {
      setLoadingOlder(false);
    }
  }, [client, conversationKey, load, oldestId]);

  return { query, loadOlder, loadingOlder };
}
