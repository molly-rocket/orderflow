import { describe, expect, it } from "vitest";
import { MINUTE_US, camera, layout } from "./testFactories";
import {
  BAR_WIDTH,
  ROW_HEIGHT,
  centerOnPrice,
  pan,
  panes,
  priceOfY,
  timeOfX,
  xOfTime,
  yOfPrice,
  zoomPrice,
  zoomTime,
} from "./viewport";

const PLOT = 800;

describe("viewport geometry", () => {
  it("stacks the main pane, stats strip, CVD pane and time axis", () => {
    expect(panes(layout())).toEqual({
      plotWidth: 800,
      main: { top: 0, height: 400 },
      stats: { top: 400, height: 30 },
      cvd: { top: 430, height: 150 },
      timeAxis: { top: 580, height: 20 },
      dom: { x: 900, width: 0 },
    });
  });

  it("gives the order book its own column right of the price axis, and the plot gets it back when off", () => {
    const withDom = panes(layout({ domWidth: 120 }));
    expect(withDom.plotWidth).toBe(680);
    expect(withDom.dom).toEqual({ x: 780, width: 120 });
    expect(panes(layout({ domWidth: 0 })).plotWidth).toBe(800);
  });

  it("maps time to x and price to y, both ways", () => {
    const cam = camera();
    expect(xOfTime(cam, PLOT, 10 * MINUTE_US)).toBe(800);
    expect(xOfTime(cam, PLOT, 9 * MINUTE_US)).toBe(700);
    expect(timeOfX(cam, PLOT, 700)).toBe(9 * MINUTE_US);
    expect(yOfPrice(cam, 100)).toBeCloseTo(0);
    expect(yOfPrice(cam, 99.5)).toBeCloseTo(50);
    expect(priceOfY(cam, 50)).toBeCloseTo(99.5);
  });

  it("zooms time around the anchor so the time under the cursor stays put", () => {
    const cam = camera();
    const anchorTime = timeOfX(cam, PLOT, 400);
    const zoomed = zoomTime(cam, PLOT, 400, 2);

    expect(zoomed.barWidth).toBe(200);
    expect(timeOfX(zoomed, PLOT, 400)).toBeCloseTo(anchorTime);
    expect(zoomTime(cam, PLOT, 400, 1000).barWidth).toBe(BAR_WIDTH.max);
    expect(zoomTime(cam, PLOT, 400, 0.0001).barWidth).toBe(BAR_WIDTH.min);
  });

  it("zooms price around the anchor so the price under the cursor stays put", () => {
    const zoomed = zoomPrice(camera(), 50, 2);

    expect(zoomed.rowHeight).toBe(20);
    expect(priceOfY(zoomed, 50)).toBeCloseTo(99.5);
    expect(zoomPrice(camera(), 50, 1000).rowHeight).toBe(ROW_HEIGHT.max);
  });

  it("pans by pixels and centres on a price", () => {
    const moved = pan(camera(), 50, 20);
    expect(moved.rightEdgeUs).toBe(10 * MINUTE_US - MINUTE_US / 2);
    expect(moved.topRow).toBe(1002);

    const centred = centerOnPrice(camera(), 120, 400);
    expect(priceOfY(centred, 200)).toBeCloseTo(120);
  });
});
