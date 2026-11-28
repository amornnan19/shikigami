export const AGENT_COLORS = [
  {
    name: "slate",
    label: "Slate",
    swatch: "bg-slate-500",
    tint: "text-slate-500",
    chip: "bg-slate-100 text-slate-800",
  },
  {
    name: "red",
    label: "Red",
    swatch: "bg-red-500",
    tint: "text-red-500",
    chip: "bg-red-100 text-red-800",
  },
  {
    name: "orange",
    label: "Orange",
    swatch: "bg-orange-500",
    tint: "text-orange-500",
    chip: "bg-orange-100 text-orange-800",
  },
  {
    name: "amber",
    label: "Amber",
    swatch: "bg-amber-500",
    tint: "text-amber-500",
    chip: "bg-amber-100 text-amber-900",
  },
  {
    name: "green",
    label: "Green",
    swatch: "bg-green-600",
    tint: "text-green-600",
    chip: "bg-green-100 text-green-800",
  },
  {
    name: "teal",
    label: "Teal",
    swatch: "bg-teal-600",
    tint: "text-teal-600",
    chip: "bg-teal-100 text-teal-800",
  },
  {
    name: "blue",
    label: "Blue",
    swatch: "bg-blue-500",
    tint: "text-blue-500",
    chip: "bg-blue-100 text-blue-800",
  },
  {
    name: "violet",
    label: "Violet",
    swatch: "bg-violet-500",
    tint: "text-violet-500",
    chip: "bg-violet-100 text-violet-800",
  },
  {
    name: "pink",
    label: "Pink",
    swatch: "bg-pink-500",
    tint: "text-pink-500",
    chip: "bg-pink-100 text-pink-800",
  },
] as const;

export function swatchClass(color: string) {
  return AGENT_COLORS.find((entry) => entry.name === color)?.swatch ?? "bg-slate-500";
}

export function chipClass(color: string) {
  return AGENT_COLORS.find((entry) => entry.name === color)?.chip ?? "bg-slate-100 text-slate-800";
}

export function tintClass(color: string) {
  return AGENT_COLORS.find((entry) => entry.name === color)?.tint ?? "text-slate-500";
}
