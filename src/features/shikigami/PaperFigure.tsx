import { cn } from "@/lib/utils";

export function PaperFigure({ tintClass, className }: { tintClass: string; className?: string }) {
  return (
    <svg
      viewBox="0 0 24 32"
      aria-hidden="true"
      className={cn("size-full overflow-visible", className)}
    >
      <g strokeLinejoin="round" strokeLinecap="round">
        <circle cx="12" cy="6" r="3.8" fill="#fbf8f1" stroke="#2f2a26" strokeWidth="0.9" />
        <path
          d="M12 10.6 L3.6 13.6 L5.4 17.2 L8.8 16 L7.8 30 L12 27.8 L16.2 30 L15.2 16 L18.6 17.2 L20.4 13.6 Z"
          fill="#fbf8f1"
          stroke="#2f2a26"
          strokeWidth="0.9"
        />
        <path
          d="M12 10.6 L20.4 13.6 L18.6 17.2 L15.2 16 L16.2 30 L12 27.8 Z"
          fill="#2f2a26"
          fillOpacity="0.07"
        />
        <path
          d="M12 10.6 L3.6 13.6 L5.4 17.2 L8.8 16 L7.8 30 L12 27.8 L16.2 30 L15.2 16 L18.6 17.2 L20.4 13.6 Z"
          fill="currentColor"
          fillOpacity="0.22"
          className={tintClass}
        />
        <path
          d="M12 10.6 L12 27.8"
          fill="none"
          stroke="#2f2a26"
          strokeOpacity="0.5"
          strokeWidth="0.6"
          strokeDasharray="1.4 1.2"
        />
      </g>
    </svg>
  );
}
