import { AgentAvatar } from "@/features/agents/AgentAvatar";
import { cn } from "@/lib/utils";
import type { GroupMember } from "./api";

export function GroupAvatars({
  members,
  limit = 2,
  className,
  figures,
}: {
  members: GroupMember[];
  limit?: number;
  className?: string;
  figures?: Record<string, number>;
}) {
  return (
    <span className="flex shrink-0 items-center">
      {members.slice(0, limit).map((member, index) => {
        const out = figures?.[member.agentId] ?? 0;
        const label = `${member.name}: ${out === 1 ? "1 paper figure out" : `${out} paper figures out`}`;
        return (
          <span
            key={member.agentId}
            className={cn("relative", index > 0 && "-ml-2", out > 0 && "z-10")}
          >
            <AgentAvatar
              name={member.name}
              color={member.color}
              className={cn("size-6 text-xs ring-2 ring-background", className)}
            />
            {out > 0 && (
              <span
                role="img"
                aria-label={label}
                title={label}
                data-figure-count={member.agentId}
                className="absolute -right-1 -bottom-1 flex min-w-4 items-center justify-center rounded-full bg-foreground px-1 text-[10px] leading-4 font-medium text-background ring-2 ring-background"
              >
                {out}
              </span>
            )}
          </span>
        );
      })}
    </span>
  );
}
