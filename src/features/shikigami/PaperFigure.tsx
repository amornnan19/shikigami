import { cn } from "@/lib/utils";

const OUTLINE =
  "M10.75 10.2 A4 4 0 1 1 13.25 10.2 L13.4 12 L20.6 13.4 L21.4 18.2 L18.2 18.6 L17.4 30 L12.9 30 L12 26.2 L11.1 30 L6.6 30 L5.8 18.6 L2.6 18.2 L3.4 13.4 L10.6 12 Z";

export function PaperFigure({ tintClass, className }: { tintClass: string; className?: string }) {
  return (
    <svg
      viewBox="0 0 24 32"
      aria-hidden="true"
      className={cn("size-full overflow-visible", className)}
    >
      <g transform="skewX(-4) translate(1.5 0)" strokeLinejoin="round" strokeLinecap="round">
        <path d={OUTLINE} fill="#fbf8f1" stroke="#2f2a26" strokeWidth="0.9" />
        <path
          d={OUTLINE}
          transform="translate(12 19) scale(0.8) translate(-12 -19)"
          fill="currentColor"
          fillOpacity="0.6"
          className={tintClass}
        />
        <path
          d="M8.2 15.4 L15.8 15"
          fill="none"
          stroke="#2f2a26"
          strokeOpacity="0.55"
          strokeWidth="0.6"
        />
      </g>
    </svg>
  );
}
