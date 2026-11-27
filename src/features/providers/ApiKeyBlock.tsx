import { useMutation, useQueryClient } from "@tanstack/react-query";
import { type FormEvent, useState } from "react";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import {
  type ApiKeyError,
  type ApiKeyStatus,
  asApiKeyError,
  removeApiKey,
  saveApiKey,
  setApiKeyEnabled,
} from "./api";

function useRefreshProviders() {
  const queryClient = useQueryClient();
  return () => queryClient.invalidateQueries({ queryKey: ["providers"] });
}

function AddApiKey({ provider }: { provider: string }) {
  const refresh = useRefreshProviders();
  const [open, setOpen] = useState(false);
  const [key, setKey] = useState("");
  const [error, setError] = useState<ApiKeyError | null>(null);

  const save = useMutation({
    mutationFn: (value: string) => saveApiKey(provider, value),
    onSuccess: async () => {
      setKey("");
      setOpen(false);
      await refresh();
    },
    onError: (failure) => setError(asApiKeyError(failure)),
  });

  function cancel() {
    setKey("");
    setError(null);
    setOpen(false);
  }

  function submit(event: FormEvent) {
    event.preventDefault();
    if (key.trim() === "" || save.isPending) return;
    setError(null);
    save.mutate(key);
  }

  if (!open) {
    return (
      <div>
        <Button variant="outline" size="sm" onClick={() => setOpen(true)}>
          Add API key
        </Button>
      </div>
    );
  }

  return (
    <form className="flex flex-col gap-2" onSubmit={submit}>
      <Label htmlFor="api-key-input">Anthropic API key</Label>
      <Input
        id="api-key-input"
        type="password"
        value={key}
        autoComplete="off"
        spellCheck={false}
        autoCorrect="off"
        autoCapitalize="off"
        aria-invalid={error !== null}
        aria-describedby="api-key-error"
        onChange={(event) => {
          setKey(event.target.value);
          setError(null);
        }}
      />
      {error && (
        <p id="api-key-error" role="alert" className="text-sm text-red-600">
          {error.message}
        </p>
      )}
      <div className="flex gap-2">
        <Button type="submit" size="sm" disabled={key.trim() === "" || save.isPending}>
          Save
        </Button>
        <Button type="button" variant="ghost" size="sm" onClick={cancel}>
          Cancel
        </Button>
      </div>
    </form>
  );
}

function RemoveApiKey({ provider }: { provider: string }) {
  const refresh = useRefreshProviders();
  const [open, setOpen] = useState(false);
  const [error, setError] = useState<ApiKeyError | null>(null);

  const remove = useMutation({
    mutationFn: () => removeApiKey(provider),
    onSuccess: async () => {
      setOpen(false);
      await refresh();
    },
    onError: (failure) => setError(asApiKeyError(failure)),
  });

  function changeOpen(next: boolean) {
    setOpen(next);
    setError(null);
  }

  return (
    <>
      <Button variant="outline" size="sm" onClick={() => changeOpen(true)}>
        Remove
      </Button>
      <Dialog open={open} onOpenChange={changeOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Remove API key</DialogTitle>
            <DialogDescription>
              Remove the saved API key? Claude Code agents will use the CLI sign-in.
            </DialogDescription>
          </DialogHeader>
          {error && (
            <p role="alert" className="text-sm text-red-600">
              {error.message}
            </p>
          )}
          <DialogFooter>
            <Button variant="ghost" onClick={() => changeOpen(false)}>
              Cancel
            </Button>
            <Button disabled={remove.isPending} onClick={() => remove.mutate()}>
              Remove
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  );
}

function SavedApiKey({ provider, status }: { provider: string; status: ApiKeyStatus }) {
  const refresh = useRefreshProviders();
  const [error, setError] = useState<ApiKeyError | null>(null);

  const toggle = useMutation({
    mutationFn: (enabled: boolean) => setApiKeyEnabled(provider, enabled),
    onSuccess: async () => {
      setError(null);
      await refresh();
    },
    onError: (failure) => setError(asApiKeyError(failure)),
  });

  return (
    <div className="flex flex-col gap-2">
      <span>API key saved (…{status.lastFour})</span>
      <div className="flex items-center gap-3">
        <div className="flex items-center gap-2">
          <Switch
            id="api-key-enabled"
            checked={status.enabled}
            disabled={toggle.isPending}
            onCheckedChange={(enabled) => toggle.mutate(enabled)}
          />
          <Label htmlFor="api-key-enabled">Use API key</Label>
        </div>
        <RemoveApiKey provider={provider} />
      </div>
      {error && (
        <p role="alert" className="text-sm text-red-600">
          {error.message}
        </p>
      )}
    </div>
  );
}

export function ApiKeyBlock({
  provider,
  status,
}: {
  provider: string;
  status: ApiKeyStatus | null;
}) {
  return (
    <section aria-label="API key" className="flex flex-col gap-2 text-sm">
      {status === null && (
        <span className="text-muted-foreground">Could not read the API key status.</span>
      )}
      {status?.saved === false && <AddApiKey provider={provider} />}
      {status?.saved === true && <SavedApiKey provider={provider} status={status} />}
    </section>
  );
}
