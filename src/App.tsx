import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";

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

  return (
    <main className="flex h-screen flex-col items-center justify-center gap-2 bg-background text-foreground">
      <h1 className="text-4xl font-semibold">Shikigami</h1>
      <p className="text-muted-foreground">Coding agents on your Mac.</p>
    </main>
  );
}

export default App;
