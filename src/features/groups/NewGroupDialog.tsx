import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
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
import { errorFor, FieldError, NAME_CODES } from "@/features/agents/AgentFields";
import { type Agent, type AgentError, asAgentError } from "@/features/agents/api";
import { nameProblem } from "@/features/agents/models";
import {
  type CreatedGroup,
  createGroup,
  GROUPS_KEY,
  pickGroupFolder,
  suggestGroupFolder,
} from "./api";
import { FOLDER_CODES, LeadField, leadOf, MEMBER_CODES, MembersField } from "./GroupFields";

function NewGroupForm({
  agents,
  onOpenChange,
  onCreated,
}: {
  agents: Agent[];
  onOpenChange: (open: boolean) => void;
  onCreated: (created: CreatedGroup) => void;
}) {
  const client = useQueryClient();
  const [name, setName] = useState("");
  const [folder, setFolder] = useState<string | null>(null);
  const [memberIds, setMemberIds] = useState<string[]>([]);
  const [leadChoice, setLeadChoice] = useState("");
  const [serverError, setServerError] = useState<AgentError | null>(null);
  const lead = leadOf(memberIds, leadChoice);

  const suggestion = useQuery({
    queryKey: ["group-folder-suggestion", name],
    queryFn: () => suggestGroupFolder(name),
    retry: false,
    placeholderData: (previous) => previous,
  });
  const pick = useMutation({
    mutationFn: pickGroupFolder,
    onSuccess: (chosen) => {
      if (chosen !== null) {
        setFolder(chosen);
        setServerError(null);
      }
    },
    onError: (error) => setServerError(asAgentError(error)),
  });
  const create = useMutation({
    mutationFn: createGroup,
    onSuccess: async (created) => {
      await client.invalidateQueries({ queryKey: GROUPS_KEY });
      onCreated(created);
      onOpenChange(false);
    },
    onError: (error) => setServerError(asAgentError(error)),
  });

  const clientNameProblem = nameProblem(name);
  const valid =
    clientNameProblem === null && memberIds.length >= 2 && lead !== "" && !create.isPending;
  const nameError = errorFor(serverError, NAME_CODES) ?? (name === "" ? null : clientNameProblem);
  const folderError = errorFor(serverError, FOLDER_CODES);
  const memberError = errorFor(serverError, MEMBER_CODES);
  const leadError = errorFor(serverError, ["lead-not-member"]);
  const known = [...NAME_CODES, ...FOLDER_CODES, ...MEMBER_CODES, "lead-not-member"];
  const formError = serverError && !known.includes(serverError.code) ? serverError.message : null;

  function submit(event: FormEvent) {
    event.preventDefault();
    if (!valid) return;
    setServerError(null);
    create.mutate({ name, folderPath: folder, memberIds, leadAgentId: lead });
  }

  return (
    <form className="flex flex-col gap-4" onSubmit={submit}>
      <div className="flex flex-col gap-2">
        <Label htmlFor="group-name">Name</Label>
        <Input
          id="group-name"
          value={name}
          autoComplete="off"
          aria-invalid={nameError !== null}
          aria-describedby="group-name-error"
          onChange={(event) => {
            setName(event.target.value);
            setServerError(null);
          }}
        />
        <FieldError id="group-name-error" message={nameError} />
      </div>

      <div className="flex flex-col gap-2">
        <Label id="group-folder-label">Folder</Label>
        <p id="group-folder-path" className="break-all font-mono text-sm">
          {folder ?? suggestion.data ?? ""}
        </p>
        <p className="text-sm text-muted-foreground">
          {folder === null
            ? "A new folder is made here. Choose a folder to work in an existing one."
            : "The agents work in this folder. Shikigami never deletes it."}
        </p>
        <div className="flex gap-2">
          <Button
            type="button"
            size="sm"
            variant="secondary"
            disabled={pick.isPending}
            onClick={() => pick.mutate()}
          >
            Choose folder…
          </Button>
          {folder !== null && (
            <Button type="button" size="sm" variant="ghost" onClick={() => setFolder(null)}>
              Use the default folder
            </Button>
          )}
        </div>
        <FieldError id="group-folder-error" message={folderError} />
      </div>

      <MembersField
        idPrefix="group"
        agents={agents}
        memberIds={memberIds}
        onChange={(next) => {
          setMemberIds(next);
          setServerError(null);
        }}
        error={memberError}
      />
      <LeadField
        id="group-lead"
        agents={agents}
        memberIds={memberIds}
        value={lead}
        onChange={(next) => {
          setLeadChoice(next);
          setServerError(null);
        }}
        error={leadError}
      />

      <FieldError id="group-form-error" message={formError} />
      <DialogFooter>
        <Button type="button" variant="ghost" onClick={() => onOpenChange(false)}>
          Cancel
        </Button>
        <Button type="submit" disabled={!valid}>
          Create
        </Button>
      </DialogFooter>
    </form>
  );
}

export function NewGroupDialog({
  agents,
  open,
  onOpenChange,
  onCreated,
}: {
  agents: Agent[];
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onCreated: (created: CreatedGroup) => void;
}) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent
        aria-describedby="new-group-description"
        className="max-h-[90vh] overflow-y-auto"
      >
        <DialogHeader>
          <DialogTitle>New group</DialogTitle>
          <DialogDescription id="new-group-description">
            Agents in a group work together in one folder.
          </DialogDescription>
        </DialogHeader>
        <NewGroupForm agents={agents} onOpenChange={onOpenChange} onCreated={onCreated} />
      </DialogContent>
    </Dialog>
  );
}
