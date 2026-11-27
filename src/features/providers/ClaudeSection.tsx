import * as RadioGroup from "@radix-ui/react-radio-group";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Check, CircleAlert, Copy, ExternalLink, RefreshCw } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { CLAUDE_CODE } from "@/features/agents/models";
import { ApiKeyBlock } from "./ApiKeyBlock";
import {
  type AuthMode,
  CLAUDE_AUTH_KEY,
  type ClaudeAuth,
  getClaudeAuth,
  setClaudeAuthMode,
} from "./api";

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

function CliStatus({ subscription }: { subscription: ClaudeAuth["subscription"] }) {
  if (!subscription.installed) {
    return (
      <div className="flex items-center gap-3 text-sm">
        <span className="flex items-center gap-1.5">
          <CircleAlert className="size-4" />
          Not installed
        </span>
        <InstallLink url={subscription.installHint} />
      </div>
    );
  }
  return (
    <div className="flex items-center gap-2 text-sm">
      <Check className="size-4" />
      <span>Ready</span>
      {subscription.version && (
        <span className="text-muted-foreground">{subscription.version}</span>
      )}
    </div>
  );
}

function SubscriptionDetail({ subscription }: { subscription: ClaudeAuth["subscription"] }) {
  if (subscription.signedIn === "yes") return <span>Signed in</span>;
  if (subscription.signedIn === "no") {
    return (
      <div className="flex flex-col gap-2">
        <span>Not signed in. Run this command in a terminal:</span>
        <SignInCommand command={subscription.signInHint} />
      </div>
    );
  }
  return <span className="text-muted-foreground">Sign-in status unknown</span>;
}

function SignInChoice({ auth }: { auth: ClaudeAuth }) {
  const queryClient = useQueryClient();
  const [adding, setAdding] = useState(false);
  const [pendingApiKey, setPendingApiKey] = useState(false);
  const choose = useMutation({
    mutationFn: setClaudeAuthMode,
    onSuccess: (_, mode) =>
      queryClient.setQueryData<ClaudeAuth>(CLAUDE_AUTH_KEY, (old) =>
        old ? { ...old, appMode: mode, appModeExplicit: true } : old,
      ),
    onSettled: async () => {
      await queryClient.invalidateQueries({ queryKey: CLAUDE_AUTH_KEY });
      await queryClient.invalidateQueries({ queryKey: ["agents"] });
    },
  });

  function change(mode: string) {
    const next = mode as AuthMode;
    if (next === "api-key" && !auth.apiKey.saved && !auth.apiKey.readFailed) {
      setPendingApiKey(true);
      setAdding(true);
      return;
    }
    setPendingApiKey(false);
    choose.mutate(next);
  }

  function changeAdding(open: boolean) {
    setAdding(open);
    if (!open) setPendingApiKey(false);
  }

  function keySaved() {
    if (pendingApiKey) choose.mutate("api-key");
  }

  return (
    <div className="flex flex-col gap-2 text-sm">
      <span id="claude-sign-in-label" className="font-medium">
        Sign in with
      </span>
      <RadioGroup.Root
        aria-labelledby="claude-sign-in-label"
        value={pendingApiKey ? "api-key" : auth.appMode}
        onValueChange={change}
        className="flex flex-col gap-3"
      >
        <div className="flex flex-col gap-2">
          <div className="flex items-center gap-2">
            <RadioGroup.Item
              id="auth-mode-subscription"
              value="subscription"
              className="size-4 rounded-full border border-foreground outline-none focus-visible:ring-2 focus-visible:ring-ring data-[state=checked]:bg-foreground"
            />
            <label htmlFor="auth-mode-subscription">Claude subscription</label>
          </div>
          <div className="pl-6">
            <SubscriptionDetail subscription={auth.subscription} />
          </div>
        </div>
        <div className="flex flex-col gap-2">
          <div className="flex items-center gap-2">
            <RadioGroup.Item
              id="auth-mode-api-key"
              value="api-key"
              className="size-4 rounded-full border border-foreground outline-none focus-visible:ring-2 focus-visible:ring-ring data-[state=checked]:bg-foreground"
            />
            <label htmlFor="auth-mode-api-key">API key</label>
          </div>
          <div className="pl-6">
            <ApiKeyBlock
              provider={CLAUDE_CODE}
              status={auth.apiKey}
              adding={adding}
              setAdding={changeAdding}
              onSaved={keySaved}
            />
          </div>
        </div>
      </RadioGroup.Root>
      {choose.isError && (
        <p role="alert" className="text-red-600">
          Could not save the choice.
        </p>
      )}
    </div>
  );
}

export function ClaudeSection() {
  const auth = useQuery({ queryKey: CLAUDE_AUTH_KEY, queryFn: getClaudeAuth, retry: false });
  return (
    <section aria-label="Claude" className="w-full max-w-md">
      <Card>
        <CardHeader>
          <CardTitle>Claude</CardTitle>
          <Button
            variant="outline"
            size="sm"
            disabled={auth.isFetching}
            onClick={() => auth.refetch()}
          >
            <RefreshCw className={auth.isFetching ? "animate-spin" : undefined} />
            Refresh
          </Button>
        </CardHeader>
        <CardContent className="flex flex-col gap-4">
          {auth.isError && <span className="text-sm">Could not check Claude.</span>}
          {auth.data && <CliStatus subscription={auth.data.subscription} />}
          {auth.data?.subscription.installed && <SignInChoice auth={auth.data} />}
        </CardContent>
      </Card>
    </section>
  );
}
