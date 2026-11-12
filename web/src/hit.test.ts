import { describe, expect, it } from "vitest";
import { ChartData } from "./chartData";
import { hitTest } from "./hit";
import { buildScene } from "./scene";
import { MINUTE_US, bar, bubble, camera, layout, row } from "./testFactories";

function chart() {
  const data = new ChartData();
  data.addHistory([bar(9 * MINUTE_US, { rows: [row("99.5", "2", "1")], cvd: "3" })], []);
  data.addBubbles([
    bubble(1, 9 * MINUTE_US + 10_000_000, { notional: "1000" }),
    bubble(2, 9 * MINUTE_US + 10_000_000, { notional: "1000", side: "sell" }),
  ]);
  return buildScene(data, camera(), layout(), {
    priceStyle: "footprint",
    showBubbles: true,
    showCvd: true,
    showDom: false,
    showHeatmap: false,
    showLastPrice: true,
    priceDecimals: 1,
    largeThreshold: 1000,
  });
}

describe("what is under the cursor", () => {
  it("is the topmost Bubble whose circle contains the point", () => {
    const scene = chart();
    const top = scene.bubbles.at(-1)!;

    const hit = hitTest(scene, top.x + 5, top.y);
    expect(hit?.kind === "bubble" && hit.shape.bubble.first_trade_id).toBe(top.bubble.first_trade_id);
  });

  it("is the Footprint cell when no Bubble covers it", () => {
    const hit = hitTest(chart(), 795, 45);
    expect(hit?.kind === "cell" && hit.cell.price).toBe("99.5");
  });

  it("is the bar in the stats strip and the CVD point in its pane", () => {
    const scene = chart();
    expect(hitTest(scene, 750, 410)?.kind).toBe("bar");
    const cvd = hitTest(scene, 760, 500);
    expect(cvd?.kind === "cvd" && cvd.point.value).toBe(3);
  });

  it("is nothing on the axes or empty areas", () => {
    const scene = chart();
    expect(hitTest(scene, 850, 45)).toBeNull();
    expect(hitTest(scene, 100, 45)).toBeNull();
  });
});
