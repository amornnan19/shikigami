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
import {
  type ApiKeyError,
  type ApiKeyStatus,
  asApiKeyError,
  CLAUDE_AUTH_KEY,
  type ClaudeAuth,
  removeApiKey,
  saveApiKey,
} from "./api";

function useAuthCache() {
  const queryClient = useQueryClient();
  return {
    show: (apiKey: ApiKeyStatus) =>
      queryClient.setQueryData<ClaudeAuth>(CLAUDE_AUTH_KEY, (old) =>
        old ? { ...old, apiKey } : old,
      ),
    refresh: async () => {
      await queryClient.invalidateQueries({ queryKey: CLAUDE_AUTH_KEY });
      await queryClient.invalidateQueries({ queryKey: ["agents"] });
    },
  };
}

function AddApiKey({
  provider,
  open,
  setOpen,
  onSaved,
}: {
  provider: string;
  open: boolean;
  setOpen: (open: boolean) => void;
  onSaved: () => void;
}) {
  const providers = useAuthCache();
  const [key, setKey] = useState("");
  const [error, setError] = useState<ApiKeyError | null>(null);

  const save = useMutation({
    mutationFn: (value: string) => saveApiKey(provider, value),
    onSuccess: (status) => {
      setKey("");
      onSaved();
      setOpen(false);
      providers.show(status);
    },
    onError: (failure) => setError(asApiKeyError(failure)),
    onSettled: async () => {
      save.reset();
      await providers.refresh();
    },
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
  const providers = useAuthCache();
  const [open, setOpen] = useState(false);
  const [error, setError] = useState<ApiKeyError | null>(null);

  const remove = useMutation({
    mutationFn: () => removeApiKey(provider),
    onSuccess: (status) => {
      setOpen(false);
      providers.show(status);
    },
    onError: (failure) => setError(asApiKeyError(failure)),
    onSettled: () => providers.refresh(),
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
              Remove the saved API key? Agents that sign in with an API key will fail until you add
              one or switch them to your Claude subscription.
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

function savedLabel(status: ApiKeyStatus) {
  if (status.readFailed) return "Could not read the saved API key.";
  return status.lastFour ? `API key saved (…${status.lastFour})` : "API key saved";
}

function SavedApiKey({ provider, status }: { provider: string; status: ApiKeyStatus }) {
  return (
    <div className="flex items-center gap-3">
      <span>{savedLabel(status)}</span>
      <RemoveApiKey provider={provider} />
    </div>
  );
}

export function ApiKeyBlock({
  provider,
  status,
  adding,
  setAdding,
  onSaved,
}: {
  provider: string;
  status: ApiKeyStatus;
  adding: boolean;
  setAdding: (adding: boolean) => void;
  onSaved: () => void;
}) {
  return (
    <section aria-label="API key" className="flex flex-col gap-2 text-sm">
      {!status.saved && !status.readFailed && (
        <AddApiKey provider={provider} open={adding} setOpen={setAdding} onSaved={onSaved} />
      )}
      {(status.saved || status.readFailed) && <SavedApiKey provider={provider} status={status} />}
    </section>
  );
}
