import { Plus } from "lucide-react";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import type { Group } from "./api";
import { GroupAvatars } from "./GroupAvatars";

export function GroupList({
  groups,
  selectedId,
  onSelect,
  onNew,
}: {
  groups: Group[];
  selectedId: string | null;
  onSelect: (id: string) => void;
  onNew: () => void;
}) {
  return (
    <section aria-label="Group chats" className="flex min-h-0 flex-1 flex-col gap-2">
      <h2 className="px-2 text-xs font-semibold text-muted-foreground">Group chats</h2>
      <Button variant="outline" size="sm" onClick={onNew}>
        <Plus />
        New group
      </Button>
      {groups.length === 0 && (
        <div className="px-2 text-sm text-muted-foreground">No group chats yet.</div>
      )}
      <ul className="flex min-h-0 flex-1 flex-col gap-1 overflow-y-auto">
        {groups.map((group) => (
          <li key={group.id}>
            <button
              type="button"
              aria-current={group.id === selectedId ? "true" : undefined}
              onClick={() => onSelect(group.id)}
              className={cn(
                "flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm outline-none hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring",
                group.id === selectedId && "bg-accent font-medium",
              )}
            >
              <GroupAvatars members={group.members} />
              <span className="min-w-0 flex-1 truncate">{group.name}</span>
              {group.attention && (
                <span
                  role="img"
                  aria-label="Needs attention"
                  className="size-2 shrink-0 rounded-full bg-amber-500"
                />
              )}
            </button>
          </li>
        ))}
      </ul>
    </section>
  );
}
