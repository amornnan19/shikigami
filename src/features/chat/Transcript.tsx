import { useLayoutEffect, useRef } from "react";
import { Button } from "@/components/ui/button";
import type { GroupMember } from "@/features/groups/api";
import type { Turn } from "./api";
import { TurnView } from "./TurnView";

const NEAR_BOTTOM_PX = 80;

export function Transcript({
  turns,
  hasMore,
  loadingOlder,
  loaded,
  onLoadOlder,
  members,
  color,
}: {
  turns: Turn[];
  hasMore: boolean;
  loadingOlder: boolean;
  loaded: boolean;
  onLoadOlder: () => void;
  members?: GroupMember[];
  color?: string;
}) {
  const scroller = useRef<HTMLDivElement>(null);
  const stick = useRef(true);
  const heightBeforeOlder = useRef<number | null>(null);

  useLayoutEffect(() => {
    const element = scroller.current;
    if (!element || turns.length === 0) return;
    if (heightBeforeOlder.current !== null) {
      element.scrollTop += element.scrollHeight - heightBeforeOlder.current;
      heightBeforeOlder.current = null;
    } else if (stick.current) {
      element.scrollTop = element.scrollHeight;
    }
  }, [turns]);

  function loadOlder() {
    heightBeforeOlder.current = scroller.current?.scrollHeight ?? null;
    onLoadOlder();
  }

  return (
    <div
      ref={scroller}
      onScroll={(event) => {
        const { scrollTop, scrollHeight, clientHeight } = event.currentTarget;
        stick.current = scrollHeight - scrollTop - clientHeight < NEAR_BOTTOM_PX;
      }}
      className="flex-1 overflow-y-auto"
    >
      <div className="mx-auto flex max-w-3xl flex-col gap-6 px-6 py-6">
        {hasMore && (
          <Button
            type="button"
            variant="ghost"
            size="sm"
            disabled={loadingOlder}
            onClick={loadOlder}
            className="self-center"
          >
            Load older messages
          </Button>
        )}
        {loaded && turns.length === 0 && (
          <p className="py-24 text-center text-muted-foreground">No messages yet.</p>
        )}
        {turns.map((turn) => (
          <TurnView key={turn.id} turn={turn} members={members} color={color} />
        ))}
      </div>
    </div>
  );
}
