import { useQuery } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import { AgentSidebar } from "@/features/agents/AgentSidebar";
import { AgentView } from "@/features/agents/AgentView";
import { listAgents } from "@/features/agents/api";
import { NewAgentDialog } from "@/features/agents/NewAgentDialog";
import { useTurnEvents } from "@/features/chat/useConversation";
import { ProvidersSection } from "@/features/providers/ProvidersSection";

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
  const agents = useQuery({ queryKey: ["agents"], queryFn: listAgents, retry: false });
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [dialogOpen, setDialogOpen] = useState(false);
  const list = agents.data ?? [];
  const selected = list.find((agent) => agent.id === selectedId) ?? null;

  return (
    <div className="flex h-screen bg-background text-foreground">
      <AgentSidebar
        agents={list}
        selectedId={selected?.id ?? null}
        onSelect={setSelectedId}
        onNew={() => setDialogOpen(true)}
      />
      {selected ? (
        <AgentView key={selected.id} agent={selected} />
      ) : (
        <main className="flex flex-1 flex-col items-center justify-center gap-6 overflow-y-auto p-6">
          <div className="flex flex-col items-center gap-2">
            <h2 className="text-4xl font-semibold">Shikigami</h2>
            <p className="text-muted-foreground">Coding agents on your Mac.</p>
          </div>
          {agents.isError && <p role="alert">Could not load agents.</p>}
          <ProvidersSection />
        </main>
      )}
      <NewAgentDialog
        open={dialogOpen}
        onOpenChange={setDialogOpen}
        onCreated={(agent) => setSelectedId(agent.id)}
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
