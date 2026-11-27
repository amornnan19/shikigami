import type * as React from "react";
import { cn } from "@/lib/utils";

function Input({ className, ...props }: React.ComponentProps<"input">) {
  return (
    <input
      className={cn(
        "h-9 w-full min-w-0 rounded-md border border-border bg-background px-3 py-1 text-sm outline-none placeholder:text-muted-foreground focus-visible:ring-2 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50 aria-invalid:border-red-500",
        className,
      )}
      {...props}
    />
  );
}

export { Input };
