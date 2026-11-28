import { AgentAvatar } from "@/features/agents/AgentAvatar";
import { tintClass } from "@/features/agents/colors";
import { PaperFigure } from "@/features/shikigami/PaperFigure";
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
                className={cn(
                  "absolute -top-2 -right-1 flex items-center gap-px rounded-full bg-background px-1 text-[10px] leading-4 font-semibold ring-1 ring-border",
                  tintClass(member.color),
                )}
              >
                <span aria-hidden="true" className="block h-3 w-2">
                  <PaperFigure tintClass={tintClass(member.color)} />
                </span>
                {out}
              </span>
            )}
          </span>
        );
      })}
    </span>
  );
}
