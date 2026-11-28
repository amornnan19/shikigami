import { Settings, TriangleAlert } from "lucide-react";
import { useState } from "react";
import { Button } from "@/components/ui/button";
import type { Agent } from "@/features/agents/api";
import type { Group } from "./api";
import { GroupAvatars } from "./GroupAvatars";
import { GroupSettingsDialog } from "./GroupSettingsDialog";

export function GroupView({
  group,
  agents,
  notice,
  onDeleted,
}: {
  group: Group;
  agents: Agent[];
  notice: string | null;
  onDeleted: () => void;
}) {
  const [settingsOpen, setSettingsOpen] = useState(false);
  return (
    <section aria-label="Group" className="flex h-screen min-w-0 flex-1 flex-col">
      <header className="flex items-center gap-3 border-b border-border px-6 py-4">
        <GroupAvatars members={group.members} limit={5} className="size-8 text-sm" />
        <div className="flex min-w-0 flex-col">
          <h2 className="truncate text-lg font-semibold">{group.name}</h2>
          <span
            id="group-header-folder"
            className="truncate font-mono text-xs text-muted-foreground"
          >
            {group.folderPath}
          </span>
        </div>
        <Button
          type="button"
          variant="ghost"
          size="icon"
          aria-label="Group settings"
          className="ml-auto"
          onClick={() => setSettingsOpen(true)}
        >
          <Settings />
        </Button>
      </header>
      {group.attention && (
        <div
          role="alert"
          aria-label="Group problem"
          className="flex items-center gap-3 border-b border-border bg-amber-50 px-6 py-3 text-sm text-amber-900"
        >
          <TriangleAlert className="size-4 shrink-0" />
          <p>{group.attention.message}</p>
        </div>
      )}
      {notice && (
        <p role="status" className="border-b border-border px-6 py-3 text-sm text-amber-800">
          {notice}
        </p>
      )}
      <div className="flex-1 overflow-y-auto px-6 py-4">
        <p className="py-24 text-center text-muted-foreground">No messages yet.</p>
      </div>
      <GroupSettingsDialog
        group={group}
        agents={agents}
        open={settingsOpen}
        onOpenChange={setSettingsOpen}
        onDeleted={onDeleted}
      />
    </section>
  );
}
