import { AgentAvatar } from "@/features/agents/AgentAvatar";
import { cn } from "@/lib/utils";
import type { GroupMember } from "./api";

export function GroupAvatars({
  members,
  limit = 2,
  className,
}: {
  members: GroupMember[];
  limit?: number;
  className?: string;
}) {
  return (
    <span aria-hidden="true" className="flex shrink-0 items-center">
      {members.slice(0, limit).map((member, index) => (
        <AgentAvatar
          key={member.agentId}
          name={member.name}
          color={member.color}
          className={cn("size-6 text-xs ring-2 ring-background", index > 0 && "-ml-2", className)}
        />
      ))}
    </span>
  );
}
