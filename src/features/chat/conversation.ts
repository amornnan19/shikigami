import type { Conversation, Item, Turn, TurnEvent } from "./api";

function withTurn(data: Conversation, turnId: string, change: (turn: Turn) => Turn): Conversation {
  if (!data.turns.some((turn) => turn.id === turnId)) {
    return data;
  }
  return {
    ...data,
    turns: data.turns.map((turn) => (turn.id === turnId ? change(turn) : turn)),
  };
}

function setText(items: Item[], itemId: string, change: (text: string) => string): Item[] {
  if (items.some((item) => item.id === itemId)) {
    return items.map((item) =>
      item.id === itemId && item.type !== "step" ? { ...item, text: change(item.text) } : item,
    );
  }
  return [...items, { type: "text", id: itemId, text: change("") }];
}

export function applyEvent(data: Conversation, event: TurnEvent): Conversation {
  switch (event.kind) {
    case "text-delta":
      return withTurn(data, event.turnId, (turn) => ({
        ...turn,
        items: setText(turn.items, event.itemId, (text) => text + event.text),
      }));
    case "text-set":
      return withTurn(data, event.turnId, (turn) => ({
        ...turn,
        items: setText(turn.items, event.itemId, () => event.text),
      }));
    case "notice":
      return withTurn(data, event.turnId, (turn) => ({
        ...turn,
        items: turn.items.some((item) => item.id === event.itemId)
          ? turn.items
          : [...turn.items, { type: "notice", id: event.itemId, text: event.text }],
      }));
    case "step-started":
      return withTurn(data, event.turnId, (turn) => ({
        ...turn,
        items: turn.items.some((item) => item.id === event.step.id)
          ? turn.items
          : [...turn.items, { ...event.step, type: "step" }],
      }));
    case "step-ended":
      return withTurn(data, event.turnId, (turn) => ({
        ...turn,
        items: turn.items.map((item) =>
          item.type === "step" && item.id === event.stepId
            ? { ...item, status: event.status, detail: event.detail }
            : item,
        ),
      }));
    case "ended":
      return withTurn(data, event.turnId, (turn) => ({
        ...turn,
        status: event.status,
        error: event.error,
        items: turn.items.map((item) =>
          item.type === "step" && item.status === "running" ? { ...item, status: "stopped" } : item,
        ),
      }));
    default:
      return data;
  }
}

function keepLongerText(fresh: Turn, known: Turn | undefined): Turn {
  if (fresh.status !== "running" || !known) {
    return fresh;
  }
  const knownTexts = new Map<string, string>();
  for (const item of known.items) {
    if (item.type === "text") knownTexts.set(item.id, item.text);
  }
  return {
    ...fresh,
    items: fresh.items.map((item) => {
      if (item.type !== "text") return item;
      const text = knownTexts.get(item.id);
      return text !== undefined && text.length > item.text.length ? { ...item, text } : item;
    }),
  };
}

export function mergeNewest(previous: Conversation | undefined, fresh: Conversation) {
  const known = new Map(previous?.turns.map((turn) => [turn.id, turn]));
  const turns = fresh.turns.map((turn) => keepLongerText(turn, known.get(turn.id)));
  const first = turns[0];
  const at = previous && first ? previous.turns.findIndex((turn) => turn.id === first.id) : -1;
  if (!previous || at < 0) {
    return turns.every((turn, index) => turn === fresh.turns[index])
      ? fresh
      : { turns, hasMore: fresh.hasMore };
  }
  return {
    turns: [...previous.turns.slice(0, at), ...turns],
    hasMore: at > 0 ? previous.hasMore : fresh.hasMore,
  };
}

export function prependOlder(current: Conversation, older: Conversation): Conversation {
  const known = new Set(current.turns.map((turn) => turn.id));
  return {
    turns: [...older.turns.filter((turn) => !known.has(turn.id)), ...current.turns],
    hasMore: older.hasMore,
  };
}
