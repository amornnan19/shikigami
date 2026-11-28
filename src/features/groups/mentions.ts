import type { GroupMember } from "./api";

const BOUNDARY_PUNCTUATION = ",.:;!?)]";
const WORD_CHARACTER = /[\p{L}\p{N}]/u;

export interface Mention {
  start: number;
  end: number;
  member: GroupMember;
}

export type ReplySegment =
  | { type: "text"; text: string }
  | { type: "hand-off"; member: GroupMember; text: string };

function markInlineCode(text: string, start: number, end: number, ignored: boolean[]) {
  const runAt = (from: number) => {
    let length = 0;
    while (from + length < end && text[from + length] === "`") length += 1;
    return length;
  };
  let at = start;
  while (at < end) {
    if (text[at] !== "`") {
      at += 1;
      continue;
    }
    const opening = runAt(at);
    let search = at + opening;
    let closing = -1;
    while (search < end) {
      if (text[search] === "`") {
        const length = runAt(search);
        if (length === opening) {
          closing = search + length;
          break;
        }
        search += length;
      } else {
        search += 1;
      }
    }
    if (closing < 0) {
      at += opening;
    } else {
      ignored.fill(true, at, closing);
      at = closing;
    }
  }
}

function lineRanges(text: string): Array<[number, number]> {
  const ranges: Array<[number, number]> = [];
  let start = 0;
  for (;;) {
    const newline = text.indexOf("\n", start);
    const end = newline < 0 ? text.length : newline;
    ranges.push([start, end]);
    if (end >= text.length) return ranges;
    start = end + 1;
  }
}

function ignoredMask(text: string): boolean[] {
  const ignored = new Array<boolean>(text.length).fill(false);
  let inFence = false;
  for (const [start, end] of lineRanges(text)) {
    if (text.slice(start, end).trimStart().startsWith("```")) {
      ignored.fill(true, start, end);
      inFence = !inFence;
    } else if (inFence) {
      ignored.fill(true, start, end);
    } else {
      markInlineCode(text, start, end, ignored);
    }
  }
  return ignored;
}

function mentionAt(
  text: string,
  ignored: boolean[],
  at: number,
  members: GroupMember[],
): Mention | null {
  if (text[at] !== "@" || ignored[at]) return null;
  const before = text[at - 1];
  if (before !== undefined && WORD_CHARACTER.test(before)) return null;
  let best: Mention | null = null;
  for (const member of members) {
    const end = at + 1 + member.name.length;
    if (member.name.length === 0 || end > text.length) continue;
    const next = text[end];
    const boundary = next === undefined || /\s/.test(next) || BOUNDARY_PUNCTUATION.includes(next);
    const matches = text.slice(at + 1, end).toLowerCase() === member.name.toLowerCase();
    const clear = !ignored.slice(at + 1, end).some(Boolean);
    if (matches && boundary && clear && (best === null || end > best.end)) {
      best = { start: at, end, member };
    }
  }
  return best;
}

export function findMentions(text: string, members: GroupMember[]): Mention[] {
  const ignored = ignoredMask(text);
  const found: Mention[] = [];
  let at = 0;
  while (at < text.length) {
    const mention = mentionAt(text, ignored, at, members);
    if (mention) {
      found.push(mention);
      at = mention.end;
    } else {
      at += 1;
    }
  }
  return found;
}

interface Pending {
  member: GroupMember;
  lines: string[];
}

export function splitReply(
  text: string,
  members: GroupMember[],
  ownAgentId: string | null,
): ReplySegment[] {
  const ignored = ignoredMask(text);
  const segments: ReplySegment[] = [];
  const state: { pending: Pending | null; before: string[] } = { pending: null, before: [] };
  for (const [start, end] of lineRanges(text)) {
    let first = start;
    while (first < end && /\s/.test(text[first] ?? "")) first += 1;
    const mention = first < end ? mentionAt(text, ignored, first, members) : null;
    if (mention && mention.member.agentId !== ownAgentId) {
      if (state.pending) {
        segments.push({
          type: "hand-off",
          member: state.pending.member,
          text: state.pending.lines.join("\n"),
        });
      }
      if (state.before.length > 0) {
        segments.push({ type: "text", text: state.before.join("\n") });
        state.before = [];
      }
      const rest = text.slice(mention.end, end).replace(/^[\s,:;]+/, "");
      state.pending = { member: mention.member, lines: [rest] };
    } else if (state.pending) {
      state.pending.lines.push(text.slice(start, end));
    } else {
      state.before.push(text.slice(start, end));
    }
  }
  if (state.pending) {
    segments.push({
      type: "hand-off",
      member: state.pending.member,
      text: state.pending.lines.join("\n"),
    });
  }
  if (state.before.length > 0) segments.push({ type: "text", text: state.before.join("\n") });
  return segments;
}

export interface Suggestion {
  start: number;
  query: string;
  members: GroupMember[];
}

export function suggestionAt(
  text: string,
  cursor: number,
  members: GroupMember[],
): Suggestion | null {
  const head = text.slice(0, cursor);
  const at = head.lastIndexOf("@");
  if (at < 0) return null;
  const before = head[at - 1];
  if (before !== undefined && !/\s/.test(before)) return null;
  const query = head.slice(at + 1);
  if (query.includes("\n")) return null;
  const lowered = query.toLowerCase();
  const matching = members.filter((member) => member.name.toLowerCase().startsWith(lowered));
  if (matching.length === 0) return null;
  if (matching.length === 1 && matching[0]?.name === query) return null;
  return { start: at, query, members: matching };
}
