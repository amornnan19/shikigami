import { cn } from "@/lib/utils";
import { swatchClass } from "./colors";

export function AgentAvatar({
  name,
  color,
  className,
}: {
  name: string;
  color: string;
  className?: string;
}) {
  const initial = (Array.from(name.trim())[0] ?? "?").toUpperCase();
  return (
    <span
      aria-hidden="true"
      className={cn(
        "flex size-8 shrink-0 items-center justify-center rounded-full text-sm font-semibold text-white",
        swatchClass(color),
        className,
      )}
    >
      {initial}
    </span>
  );
}
