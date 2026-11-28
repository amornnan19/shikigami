import type { GroupMember } from "./api";

const BOUNDARY_PUNCTUATION = ",.:;!?)]";
const WORD_CHARACTER = /[\p{Alphabetic}\p{N}]/u;

export interface Mention {
  start: number;
  end: number;
  member: GroupMember;
}

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

function fenceOf(line: string): { marker: string; length: number; rest: string } | null {
  const trimmed = line.trimStart();
  const marker = trimmed[0];
  if (marker !== "`" && marker !== "~") return null;
  let length = 0;
  while (trimmed[length] === marker) length += 1;
  return length >= 3 ? { marker, length, rest: trimmed.slice(length) } : null;
}

function ignoredMask(text: string): boolean[] {
  const ignored = new Array<boolean>(text.length).fill(false);
  let open: { marker: string; length: number } | null = null;
  for (const [start, end] of lineRanges(text)) {
    const fence = fenceOf(text.slice(start, end));
    if (open) {
      ignored.fill(true, start, end);
      if (
        fence &&
        fence.marker === open.marker &&
        fence.length >= open.length &&
        fence.rest.trim() === ""
      ) {
        open = null;
      }
    } else if (fence) {
      ignored.fill(true, start, end);
      open = { marker: fence.marker, length: fence.length };
    } else {
      markInlineCode(text, start, end, ignored);
    }
  }
  return ignored;
}

function lastCodePoint(text: string, at: number): string | undefined {
  return Array.from(text.slice(Math.max(0, at - 2), at)).at(-1);
}

function mentionAt(
  text: string,
  ignored: boolean[],
  at: number,
  members: GroupMember[],
): Mention | null {
  if (text[at] !== "@" || ignored[at]) return null;
  const before = lastCodePoint(text, at);
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
  const trimmed = lowered.trimEnd();
  if (trimmed !== lowered && members.some((member) => member.name.toLowerCase() === trimmed)) {
    return null;
  }
  const matching = members.filter((member) => member.name.toLowerCase().startsWith(lowered));
  if (matching.length === 0) return null;
  const exact = matching.filter((member) => member.name.toLowerCase() === lowered);
  const rest = matching.filter((member) => member.name.toLowerCase() !== lowered);
  return { start: at, query, members: [...exact, ...rest] };
}
