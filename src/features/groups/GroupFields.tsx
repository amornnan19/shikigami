import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { AgentAvatar } from "@/features/agents/AgentAvatar";
import { FieldError } from "@/features/agents/AgentFields";
import type { Agent } from "@/features/agents/api";

export const MEMBER_CODES = ["members-too-few", "member-not-found"];
export const FOLDER_CODES = ["folder-refused", "folder-invalid", "folder-failed"];

export function leadOf(memberIds: string[], lead: string) {
  return memberIds.includes(lead) ? lead : (memberIds[0] ?? "");
}

export function MembersField({
  idPrefix,
  agents,
  memberIds,
  onChange,
  error,
}: {
  idPrefix: string;
  agents: Agent[];
  memberIds: string[];
  onChange: (memberIds: string[]) => void;
  error: string | null;
}) {
  function toggle(agentId: string, checked: boolean) {
    onChange(
      checked ? [...memberIds, agentId] : memberIds.filter((memberId) => memberId !== agentId),
    );
  }
  return (
    <fieldset className="flex flex-col gap-2" aria-describedby={`${idPrefix}-members-error`}>
      <legend className="text-sm font-medium">Members</legend>
      <ul className="flex max-h-40 flex-col gap-1 overflow-y-auto rounded-md border border-border p-2">
        {agents.length === 0 && (
          <li className="text-sm text-muted-foreground">Create agents first.</li>
        )}
        {agents.map((agent) => (
          <li key={agent.id}>
            <label className="flex items-center gap-2 text-sm">
              <input
                type="checkbox"
                className="size-4"
                checked={memberIds.includes(agent.id)}
                onChange={(event) => toggle(agent.id, event.target.checked)}
              />
              <AgentAvatar name={agent.name} color={agent.color} className="size-5 text-xs" />
              <span className="min-w-0 flex-1 truncate">{agent.name}</span>
            </label>
          </li>
        ))}
      </ul>
      <FieldError id={`${idPrefix}-members-error`} message={error} />
    </fieldset>
  );
}

export function LeadField({
  id,
  agents,
  memberIds,
  value,
  onChange,
  error,
}: {
  id: string;
  agents: Agent[];
  memberIds: string[];
  value: string;
  onChange: (agentId: string) => void;
  error: string | null;
}) {
  const members = memberIds.flatMap((memberId) => agents.filter((agent) => agent.id === memberId));
  return (
    <div className="flex flex-col gap-2">
      <Label htmlFor={id}>Lead</Label>
      <Select value={value} onValueChange={onChange} disabled={members.length === 0}>
        <SelectTrigger id={id} aria-describedby={`${id}-error`}>
          <SelectValue placeholder="Choose members first" />
        </SelectTrigger>
        <SelectContent>
          {members.map((agent) => (
            <SelectItem key={agent.id} value={agent.id}>
              {agent.name}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
      <FieldError id={`${id}-error`} message={error} />
    </div>
  );
}
