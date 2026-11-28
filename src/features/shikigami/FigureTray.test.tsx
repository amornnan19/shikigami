import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { FigureTrayView } from "./FigureTray";
import type { TrayFigure } from "./subagents";

const working = (id: string, type = "Explore"): TrayFigure => ({ id, type, phase: "working" });

function view(figures: TrayFigure[], reduced: boolean, running = figures.length) {
  return renderToStaticMarkup(
    <FigureTrayView
      figures={figures}
      running={running}
      tint="text-red-500"
      reduced={reduced}
      anchor={36}
    />,
  );
}

describe("FigureTrayView", () => {
  it("draws one labelled figure per subagent", () => {
    const html = view([working("a", "Explore"), working("b", "Plan")], false);
    expect(html.match(/data-figure="/g)).toHaveLength(2);
    expect(html).toContain('aria-label="Explore"');
    expect(html).toContain('title="Plan"');
    expect(html).toContain("paper-figure-working");
    expect(html).not.toContain("data-figure-badge");
  });

  it("marks the phase of a figure that is leaving", () => {
    const html = view(
      [
        { id: "a", type: "x", phase: "folding" },
        { id: "b", type: "y", phase: "dissolving" },
      ],
      false,
      0,
    );
    expect(html).toContain("paper-figure-folding");
    expect(html).toContain("paper-figure-dissolving");
    expect(html).toContain("text-red-600");
  });

  it("shows three figures at most and the rest as +N on the last one", () => {
    const html = view(
      [working("a"), working("b"), working("c"), working("d"), working("e")],
      false,
    );
    expect(html.match(/data-figure="/g)).toHaveLength(3);
    expect(html.match(/data-figure-more/g)).toHaveLength(1);
    expect(html).toContain("+2");
  });

  it("shows nothing when no figure is out", () => {
    expect(view([], false, 0)).toBe("");
  });

  it("shows a static stacked badge with the count and no animated figure under reduced motion", () => {
    const html = view([working("a"), working("b")], true);
    expect(html).toContain("data-figure-badge");
    expect(html).toContain('aria-label="2 paper figures out"');
    expect(html).not.toContain("data-figure=");
    expect(html).not.toContain("paper-figure");
    expect(html).not.toContain("data-figure-tray");
  });

  it("shows no badge under reduced motion when nothing is running", () => {
    expect(view([{ id: "a", type: "x", phase: "folding" }], true, 0)).toBe("");
  });
});
