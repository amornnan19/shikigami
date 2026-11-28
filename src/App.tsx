import { useQuery, useQueryClient } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import { AgentSidebar } from "@/features/agents/AgentSidebar";
import { AgentView } from "@/features/agents/AgentView";
import { type Deleted, listAgents } from "@/features/agents/api";
import { NewAgentDialog } from "@/features/agents/NewAgentDialog";
import { useTurnEvents } from "@/features/chat/useConversation";
import {
  type CreatedGroup,
  GROUPS_KEY,
  listGroups,
  SHARED_FOLDER_NOTICE,
} from "@/features/groups/api";
import { GroupView } from "@/features/groups/GroupView";
import { NewGroupDialog } from "@/features/groups/NewGroupDialog";
import { ClaudeSection } from "@/features/providers/ClaudeSection";

type AppStatus = { status: "ok" } | { status: "error"; message: string; data_dir: string };

function ErrorScreen({ message, dataDir }: { message: string; dataDir: string }) {
  return (
    <main className="flex h-screen flex-col items-center justify-center gap-3 bg-background p-8 text-center text-foreground">
      <h1 className="text-3xl font-semibold">Shikigami could not start</h1>
      <p role="alert" className="max-w-xl break-words">
        {message}
      </p>
      <p className="max-w-xl break-all text-sm text-muted-foreground">
        Data folder: <span>{dataDir}</span>
      </p>
    </main>
  );
}

function Workspace() {
  useTurnEvents();
  const agents = useQuery({
    queryKey: ["agents"],
    queryFn: listAgents,
    retry: false,
    refetchOnWindowFocus: true,
  });
  const groups = useQuery({
    queryKey: GROUPS_KEY,
    queryFn: listGroups,
    retry: false,
    refetchOnWindowFocus: true,
  });
  const client = useQueryClient();
  const agentsUpdatedAt = agents.dataUpdatedAt;
  useEffect(() => {
    if (agentsUpdatedAt > 0) void client.invalidateQueries({ queryKey: GROUPS_KEY });
  }, [agentsUpdatedAt, client]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [selectedGroupId, setSelectedGroupId] = useState<string | null>(null);
  const [dialogOpen, setDialogOpen] = useState(false);
  const [groupDialogOpen, setGroupDialogOpen] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const list = agents.data ?? [];
  const groupList = groups.data ?? [];
  const selected = list.find((agent) => agent.id === selectedId) ?? null;
  const selectedGroup = groupList.find((group) => group.id === selectedGroupId) ?? null;

  function select(id: string) {
    setNotice(null);
    setSelectedGroupId(null);
    setSelectedId(id);
  }

  function selectGroup(id: string) {
    setNotice(null);
    setSelectedId(null);
    setSelectedGroupId(id);
  }

  function groupCreated(created: CreatedGroup) {
    selectGroup(created.group.id);
    if (created.sharedFolderWith.length > 0) setNotice(SHARED_FOLDER_NOTICE);
  }

  function deleted(outcome: Deleted, askedForTrash: boolean) {
    setSelectedId(null);
    setNotice(
      askedForTrash && outcome.folderKept
        ? `The agent was deleted, but its folder was not moved to the Trash. ${outcome.reason ?? ""}`.trim()
        : null,
    );
  }

  return (
    <div className="flex h-screen bg-background text-foreground">
      <AgentSidebar
        agents={list}
        groups={groupList}
        selectedId={selected?.id ?? null}
        selectedGroupId={selectedGroup?.id ?? null}
        onSelect={select}
        onSelectGroup={selectGroup}
        onNew={() => setDialogOpen(true)}
        onNewGroup={() => setGroupDialogOpen(true)}
      />
      {selected ? (
        <AgentView key={selected.id} agent={selected} onDeleted={deleted} />
      ) : selectedGroup ? (
        <GroupView
          key={selectedGroup.id}
          group={selectedGroup}
          agents={list}
          notice={notice}
          onDeleted={() => {
            setSelectedGroupId(null);
            setNotice(null);
          }}
        />
      ) : (
        <main className="flex flex-1 flex-col items-center justify-center gap-6 overflow-y-auto p-6">
          <div className="flex flex-col items-center gap-2">
            <h2 className="text-4xl font-semibold">Shikigami</h2>
            <p className="text-muted-foreground">Coding agents on your Mac.</p>
          </div>
          {agents.isError && <p role="alert">Could not load agents.</p>}
          {groups.isError && <p role="alert">Could not load group chats.</p>}
          {notice && (
            <p role="status" className="max-w-md text-center text-sm text-amber-800">
              {notice}
            </p>
          )}
          <ClaudeSection />
        </main>
      )}
      <NewAgentDialog
        open={dialogOpen}
        onOpenChange={setDialogOpen}
        onCreated={(agent) => select(agent.id)}
      />
      <NewGroupDialog
        agents={list}
        open={groupDialogOpen}
        onOpenChange={setGroupDialogOpen}
        onCreated={groupCreated}
      />
    </div>
  );
}

function App() {
  const [status, setStatus] = useState<AppStatus | null>(null);

  useEffect(() => {
    invoke<AppStatus>("app_status").then(setStatus, (error) =>
      setStatus({ status: "error", message: String(error), data_dir: "" }),
    );
  }, []);

  if (status === null) {
    return null;
  }

  if (status.status === "error") {
    return <ErrorScreen message={status.message} dataDir={status.data_dir} />;
  }

  return <Workspace />;
}

export default App;
