import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback, useEffect, useState } from "react";
import { type Conversation, getConversation, onTurnEvent } from "./api";
import { applyEvent, mergeNewest, prependOlder } from "./conversation";

const key = (agentId: string) => ["conversation", agentId];

export function useTurnEvents() {
  const client = useQueryClient();
  useEffect(() => {
    const stop = onTurnEvent((event) => {
      if (event.kind === "started" || event.kind === "ended") {
        void client.invalidateQueries({ queryKey: key(event.agentId) });
        void client.invalidateQueries({ queryKey: ["agents"] });
      }
      client.setQueryData<Conversation>(key(event.agentId), (data) =>
        data ? applyEvent(data, event) : data,
      );
    });
    return () => {
      void stop.then((unlisten) => unlisten());
    };
  }, [client]);
}

export function useConversation(agentId: string) {
  const client = useQueryClient();
  const [loadingOlder, setLoadingOlder] = useState(false);
  const query = useQuery({
    queryKey: key(agentId),
    queryFn: async () =>
      mergeNewest(client.getQueryData<Conversation>(key(agentId)), await getConversation(agentId)),
    retry: false,
  });
  const oldestId = query.data?.turns[0]?.id;

  const loadOlder = useCallback(async () => {
    if (!oldestId) return;
    setLoadingOlder(true);
    try {
      const older = await getConversation(agentId, oldestId);
      client.setQueryData<Conversation>(key(agentId), (data) =>
        data ? prependOlder(data, older) : data,
      );
    } finally {
      setLoadingOlder(false);
    }
  }, [agentId, client, oldestId]);

  return { query, loadOlder, loadingOlder };
}
