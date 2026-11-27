export const AGENT_COLORS = [
  { name: "slate", label: "Slate", swatch: "bg-slate-500" },
  { name: "red", label: "Red", swatch: "bg-red-500" },
  { name: "orange", label: "Orange", swatch: "bg-orange-500" },
  { name: "amber", label: "Amber", swatch: "bg-amber-500" },
  { name: "green", label: "Green", swatch: "bg-green-600" },
  { name: "teal", label: "Teal", swatch: "bg-teal-600" },
  { name: "blue", label: "Blue", swatch: "bg-blue-500" },
  { name: "violet", label: "Violet", swatch: "bg-violet-500" },
  { name: "pink", label: "Pink", swatch: "bg-pink-500" },
] as const;

export function swatchClass(color: string) {
  return AGENT_COLORS.find((entry) => entry.name === color)?.swatch ?? "bg-slate-500";
}
