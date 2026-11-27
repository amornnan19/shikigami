import { useQuery } from "@tanstack/react-query";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Check, CircleAlert, CircleHelp, Copy, ExternalLink, RefreshCw } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { listProviders, type ProviderStatus } from "./api";

function VersionText({ version }: { version: string | null }) {
  return version ? <span className="text-muted-foreground">{version}</span> : null;
}

const FEEDBACK_MS = 2000;

type CopyState = "idle" | "copied" | "failed";

function SignInCommand({ command }: { command: string }) {
  const [copyState, setCopyState] = useState<CopyState>("idle");
  const timer = useRef<ReturnType<typeof setTimeout>>(undefined);

  useEffect(() => () => clearTimeout(timer.current), []);

  async function copy() {
    let next: CopyState = "copied";
    try {
      await navigator.clipboard.writeText(command);
    } catch {
      next = "failed";
    }
    setCopyState(next);
    clearTimeout(timer.current);
    timer.current = setTimeout(() => setCopyState("idle"), FEEDBACK_MS);
  }

  return (
    <div className="flex items-center gap-2">
      <code className="rounded-md bg-muted px-2 py-1 font-mono text-sm">{command}</code>
      <Button variant="ghost" size="sm" aria-label={`Copy ${command}`} onClick={copy}>
        <Copy />
        {copyState === "copied" && "Copied"}
        {copyState === "failed" && "Copy failed"}
      </Button>
    </div>
  );
}

function InstallLink({ url }: { url: string }) {
  const [failed, setFailed] = useState(false);

  async function open() {
    try {
      await openUrl(url);
      setFailed(false);
    } catch {
      setFailed(true);
    }
  }

  return (
    <>
      <Button variant="outline" size="sm" onClick={open}>
        <ExternalLink />
        Install
      </Button>
      {failed && (
        <span role="alert" className="text-muted-foreground">
          Could not open the browser. Go to {url}
        </span>
      )}
    </>
  );
}

function ProviderRow({ provider }: { provider: ProviderStatus }) {
  return (
    <li className="flex flex-col gap-2 border-t border-border pt-4 first:border-t-0 first:pt-0">
      <span className="font-medium">{provider.name}</span>
      {!provider.installed && (
        <div className="flex items-center gap-3 text-sm">
          <span className="flex items-center gap-1.5">
            <CircleAlert className="size-4" />
            Not installed
          </span>
          <InstallLink url={provider.installHint} />
        </div>
      )}
      {provider.installed && provider.signedIn === "yes" && (
        <div className="flex items-center gap-2 text-sm">
          <Check className="size-4" />
          <span>Ready</span>
          <VersionText version={provider.version} />
        </div>
      )}
      {provider.installed && provider.signedIn === "no" && (
        <div className="flex flex-col gap-2 text-sm">
          <div className="flex items-center gap-2">
            <CircleAlert className="size-4" />
            <span>Not signed in</span>
            <VersionText version={provider.version} />
          </div>
          <span className="text-muted-foreground">Run this command in a terminal:</span>
          <SignInCommand command={provider.signInHint} />
        </div>
      )}
      {provider.installed && provider.signedIn === "unknown" && (
        <div className="flex items-center gap-2 text-sm">
          <CircleHelp className="size-4" />
          <span>Sign-in status unknown</span>
          <VersionText version={provider.version} />
        </div>
      )}
    </li>
  );
}

export function ProvidersSection() {
  const providers = useQuery({ queryKey: ["providers"], queryFn: listProviders, retry: false });
  return (
    <section aria-label="Providers" className="w-full max-w-md">
      <Card>
        <CardHeader>
          <CardTitle>Providers</CardTitle>
          <Button
            variant="outline"
            size="sm"
            disabled={providers.isFetching}
            onClick={() => providers.refetch()}
          >
            <RefreshCw className={providers.isFetching ? "animate-spin" : undefined} />
            Refresh
          </Button>
        </CardHeader>
        <CardContent>
          {providers.isError && <span className="text-sm">Could not check providers.</span>}
          <ul className="flex flex-col gap-4">
            {providers.data?.map((provider) => (
              <ProviderRow key={provider.id} provider={provider} />
            ))}
          </ul>
        </CardContent>
      </Card>
    </section>
  );
}
