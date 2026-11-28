import { useMutation, useQueryClient } from "@tanstack/react-query";
import { type ReactNode, useState } from "react";
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
import { deleteGroup, GROUPS_KEY, type Group, updateGroup } from "./api";
import { LeadField, leadOf, MEMBER_CODES, MembersField } from "./GroupFields";

function Section({ label, children }: { label: string; children: ReactNode }) {
  return (
    <section aria-label={label} className="flex flex-col gap-3 border-t border-border pt-4">
      <h3 className="text-sm font-semibold">{label}</h3>
      {children}
    </section>
  );
}

function useGroupsRefresh() {
  const client = useQueryClient();
  return () => client.invalidateQueries({ queryKey: GROUPS_KEY });
}

function NameSection({ group }: { group: Group }) {
  const refresh = useGroupsRefresh();
  const [name, setName] = useState(group.name);
  const [error, setError] = useState<AgentError | null>(null);
  const save = useMutation({
    mutationFn: () => updateGroup(group.id, { name }),
    onSuccess: refresh,
    onError: (failure) => setError(asAgentError(failure)),
  });
  const valid = nameProblem(name) === null && name.trim() !== group.name && !save.isPending;
  const nameError = errorFor(error, NAME_CODES);
  const otherError = error && !nameError ? error.message : null;

  return (
    <Section label="Name">
      <div className="flex flex-col gap-2">
        <Label htmlFor="group-settings-name">Name</Label>
        <Input
          id="group-settings-name"
          value={name}
          autoComplete="off"
          aria-invalid={nameError !== null}
          aria-describedby="group-settings-name-error"
          onChange={(event) => {
            setName(event.target.value);
            setError(null);
          }}
        />
        <FieldError id="group-settings-name-error" message={nameError} />
      </div>
      <FieldError id="group-settings-name-form-error" message={otherError} />
      <div>
        <Button type="button" size="sm" disabled={!valid} onClick={() => save.mutate()}>
          Save name
        </Button>
      </div>
    </Section>
  );
}

function MembersSection({ group, agents }: { group: Group; agents: Agent[] }) {
  const refresh = useGroupsRefresh();
  const saved = group.members.map((member) => member.agentId);
  const [memberIds, setMemberIds] = useState(saved);
  const [leadChoice, setLeadChoice] = useState(group.leadAgentId ?? "");
  const [error, setError] = useState<AgentError | null>(null);
  const lead = leadOf(memberIds, leadChoice);
  const save = useMutation({
    mutationFn: () => updateGroup(group.id, { memberIds, leadAgentId: lead }),
    onSuccess: refresh,
    onError: (failure) => setError(asAgentError(failure)),
  });
  const changed = memberIds.join() !== saved.join() || lead !== (group.leadAgentId ?? "");
  const valid = changed && memberIds.length >= 2 && lead !== "" && !save.isPending;
  const memberError = errorFor(error, MEMBER_CODES);
  const leadError = errorFor(error, ["lead-not-member"]);
  const otherError = error && !memberError && !leadError ? error.message : null;

  return (
    <Section label="Members">
      <MembersField
        idPrefix="group-settings"
        agents={agents}
        memberIds={memberIds}
        onChange={(next) => {
          setMemberIds(next);
          setError(null);
        }}
        error={memberError}
      />
      <LeadField
        id="group-settings-lead"
        agents={agents}
        memberIds={memberIds}
        value={lead}
        onChange={(next) => {
          setLeadChoice(next);
          setError(null);
        }}
        error={leadError}
      />
      <FieldError id="group-settings-members-form-error" message={otherError} />
      <div>
        <Button type="button" size="sm" disabled={!valid} onClick={() => save.mutate()}>
          Save members
        </Button>
      </div>
    </Section>
  );
}

function DeleteGroupDialog({
  group,
  open,
  onOpenChange,
  onDeleted,
}: {
  group: Group;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onDeleted: () => void;
}) {
  const refresh = useGroupsRefresh();
  const remove = useMutation({
    mutationFn: () => deleteGroup(group.id),
    onSuccess: async () => {
      await refresh();
      onDeleted();
    },
    onError: refresh,
  });
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent aria-describedby="delete-group-description">
        <DialogHeader>
          <DialogTitle>Delete {group.name}?</DialogTitle>
          <DialogDescription id="delete-group-description">
            Its messages are removed from Shikigami. The folder is kept.
          </DialogDescription>
        </DialogHeader>
        {remove.isError && (
          <p role="alert" className="text-sm text-red-600">
            {asAgentError(remove.error).message}
          </p>
        )}
        <DialogFooter>
          <Button type="button" variant="ghost" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button
            type="button"
            variant="destructive"
            disabled={remove.isPending}
            onClick={() => remove.mutate()}
          >
            Delete
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function DeleteSection({ group, onDeleted }: { group: Group; onDeleted: () => void }) {
  const [confirming, setConfirming] = useState(false);
  return (
    <Section label="Delete">
      <div>
        <Button type="button" size="sm" variant="destructive" onClick={() => setConfirming(true)}>
          Delete group
        </Button>
      </div>
      <DeleteGroupDialog
        group={group}
        open={confirming}
        onOpenChange={setConfirming}
        onDeleted={onDeleted}
      />
    </Section>
  );
}

export function GroupSettingsDialog({
  group,
  agents,
  open,
  onOpenChange,
  onDeleted,
}: {
  group: Group;
  agents: Agent[];
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onDeleted: () => void;
}) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent
        aria-describedby="group-settings-description"
        className="max-h-[90vh] max-w-2xl overflow-y-auto"
      >
        <DialogHeader>
          <DialogTitle>Group settings</DialogTitle>
          <DialogDescription id="group-settings-description">
            Changes keep the folder and its files.
          </DialogDescription>
        </DialogHeader>
        <NameSection group={group} />
        <MembersSection
          key={group.members.map((member) => member.agentId).join()}
          group={group}
          agents={agents}
        />
        <DeleteSection group={group} onDeleted={onDeleted} />
      </DialogContent>
    </Dialog>
  );
}
