import { describe, expect, it } from "vitest";
import { BookData } from "./bookData";
import { ChartData } from "./chartData";
import { BUBBLE, type SceneOptions, bubbleRadius, buildScene, footprintMode, niceMultiple } from "./scene";
import { DAY_US, MINUTE_US, bar, book, bubble, camera, frame, layout, row } from "./testFactories";
import type { Layout } from "./viewport";

const ALL: SceneOptions = {
  priceStyle: "footprint",
  showBubbles: true,
  showCvd: true,
  showDom: true,
  showHeatmap: true,
  showLastPrice: true,
  priceDecimals: 1,
  largeThreshold: 1000,
};

function scene(
  data: ChartData,
  options: Partial<SceneOptions> = {},
  cam = camera(),
  bookData: BookData | null = null,
  layoutExtra: Partial<Layout> = {},
) {
  return buildScene(data, cam, layout(layoutExtra), { ...ALL, ...options }, bookData);
}

function ohlcBar(start: number, open: string, high: string, low: string, close: string) {
  return bar(start, { open, high, low, close });
}

describe("price styles", () => {
  it("candles use exact open/high/low/close and colour by direction", () => {
    const data = new ChartData();
    data.addHistory([ohlcBar(9 * MINUTE_US, "99.6", "99.9", "99.2", "99.4"), bar(8 * MINUTE_US, { open: null })], []);

    const built = scene(data, { priceStyle: "candles" });
    expect(built.candles).toHaveLength(1);
    expect(built.candles[0]).toMatchObject({ x: 700, width: 100, up: false });
    const c = built.candles[0]!;
    expect([c.open, c.close, c.highY, c.lowY].map(Math.round)).toEqual([40, 60, 10, 80]);
    expect(built.columns.every((col) => col.cells.length === 0)).toBe(true);
    expect(built.line).toEqual([]);
  });

  it("line joins closes at bar centres and its last point follows the live close", () => {
    const data = new ChartData();
    data.addHistory([ohlcBar(8 * MINUTE_US, "99", "99", "99", "99.4"), ohlcBar(9 * MINUTE_US, "99", "99", "99", "99.6")], []);
    data.applyLive({ type: "bar", bar: ohlcBar(9 * MINUTE_US, "99", "99", "99", "99.8"), closed: false });

    const built = scene(data, { priceStyle: "line" });
    expect(built.line.map((p) => [p.x, Math.round(p.y)])).toEqual([
      [650, 60],
      [750, 20],
    ]);
    expect(built.candles).toEqual([]);
  });

  it("live price tag sits at the newest close and turns red after a down move", () => {
    const data = new ChartData();
    data.addHistory([ohlcBar(8 * MINUTE_US, "99", "99", "99", "99.6"), ohlcBar(9 * MINUTE_US, "99", "99", "99", "99.4")], []);

    expect(scene(data).lastPrice).toMatchObject({ label: "99.4", up: false });
    expect(Math.round(scene(data).lastPrice!.y)).toBe(60);

    data.applyLive({ type: "bar", bar: ohlcBar(9 * MINUTE_US, "99", "99", "99", "99.7"), closed: false });
    expect(scene(data).lastPrice).toMatchObject({ label: "99.7", up: true });
    expect(scene(data, { showLastPrice: false }).lastPrice).toBeNull();
  });

  it("hidden style draws no price but keeps the live price tag", () => {
    const data = new ChartData();
    data.addHistory([ohlcBar(9 * MINUTE_US, "99.6", "99.9", "99.2", "99.4")], []);

    const built = scene(data, { priceStyle: "hidden" });
    expect(built.candles).toEqual([]);
    expect(built.line).toEqual([]);
    expect(built.columns[0]?.cells).toEqual([]);
    expect(built.lastPrice?.label).toBe("99.4");
  });

  it("bubbles are fainter over footprint than over candles", () => {
    const data = new ChartData();
    data.addBubbles([bubble(1, 9 * MINUTE_US)]);
    expect(scene(data).bubbles[0]?.alpha).toBe(0.55);
    expect(scene(data, { priceStyle: "candles" }).bubbles[0]?.alpha).toBe(1);
  });
});

describe("order book ladder", () => {
  const dom = { domWidth: 120 };

  it("rows line up with chart rows for the same price and scale to the largest size", () => {
    const bookData = new BookData();
    bookData.applyBook({ type: "book", book: book() });

    const ladder = scene(new ChartData(), {}, camera(), bookData, dom).ladder!;
    expect(ladder.status).toBe("live");
    expect(ladder.header).toBe("bid 99.5 · ask 99.6 · spread 0.1");
    expect(ladder.rows.map((r) => [r.price, Math.round(r.y), r.height, r.bidShare, r.askShare])).toEqual([
      ["99.5", 40, 10, 0.75, 0],
      ["99.6", 30, 10, 0, 1],
    ]);
  });

  it("marks prices outside the known range as unknown and dims when resyncing", () => {
    const bookData = new BookData();
    bookData.applyBook({ type: "book", book: book() });
    const live = scene(new ChartData(), {}, camera(), bookData, dom).ladder!;
    expect(live.unknown.map((u) => [Math.round(u.y0), Math.round(u.y1)])).toEqual([
      [0, 20],
      [60, 400],
    ]);

    const resync = book({ status: "resyncing", rows: [], known_low: null, known_high: null, best_bid: null, best_ask: null });
    bookData.applyBook({ type: "book", book: resync });
    const resyncing = scene(new ChartData(), {}, camera(), bookData, dom).ladder!;
    expect(resyncing.status).toBe("resyncing");
    expect(resyncing.rows).toEqual([]);
    expect(resyncing.unknown.map((u) => [u.y0, u.y1])).toEqual([[0, 400]]);
    expect(scene(new ChartData(), { showDom: false }, camera(), bookData, dom).ladder).toBeNull();
  });
});

describe("heatmap", () => {
  function frames() {
    const bookData = new BookData();
    bookData.applyBook({
      type: "heatmap_history",
      frames: [frame(8 * MINUTE_US, { low: "99.4", sizes: [1, 4, 0, 16] }), frame(9 * MINUTE_US, { low: "99.5", sizes: [4, 4] })],
    });
    return bookData;
  }

  it("cells span from each frame to the next and scale by the 95th percentile", () => {
    const heatmap = scene(new ChartData(), {}, camera(), frames()).heatmap!;
    expect(heatmap.cells.map((c) => [c.x, Math.round(c.y), c.w, c.h, c.v])).toEqual([
      [600, 50, 100, 10, 0.25],
      [600, 40, 100, 10, 0.5],
      [600, 20, 100, 10, 1],
      [700, 40, 100, 10, 0.5],
      [700, 30, 100, 10, 0.5],
    ]);
  });

  it("caps a frame's span at twice the expected interval and marks the rest unknown", () => {
    const bookData = new BookData();
    bookData.applyBook({
      type: "heatmap_history",
      frames: [
        frame(8 * MINUTE_US, { low: "99.5", sizes: [4] }),
        frame(8 * MINUTE_US + 1_000_000, { low: "99.5", sizes: [4] }),
        frame(8 * MINUTE_US + 2_000_000, { low: "99.5", sizes: [4] }),
        frame(9 * MINUTE_US + 30_000_000, { low: "99.5", sizes: [4] }),
      ],
    });

    const heatmap = scene(new ChartData(), {}, camera(), bookData).heatmap!;
    const third = heatmap.cells[2]!;
    expect([third.x, Math.round(third.w * 60)]).toEqual([600 + 200 / 60, 200]);
    expect(heatmap.unknown).toContainEqual({ x0: 600 + 400 / 60, x1: 750, y0: 0, y1: 400 });
  });

  it("merges frames narrower than a pixel into one cell per column, keeping the latest", () => {
    const bookData = new BookData();
    const frames = [];
    for (let i = 0; i < 60; i++) frames.push(frame(9 * MINUTE_US + i * 1_000_000, { low: "99.5", sizes: [i + 1] }));
    bookData.applyBook({ type: "heatmap_history", frames });

    const heatmap = scene(new ChartData(), {}, camera({ barWidth: 6 }), bookData).heatmap!;
    expect(heatmap.cells.length).toBeLessThanOrEqual(7);
    expect(Math.max(...heatmap.cells.map((c) => c.v))).toBe(1);
    expect(heatmap.cells.at(-1)?.x).toBeGreaterThan(heatmap.cells[0]!.x);
  });

  it("marks time before its first frame and prices outside each frame as unknown", () => {
    const heatmap = scene(new ChartData(), {}, camera(), frames()).heatmap!;
    expect(heatmap.unknown.map((u) => [u.x0, u.x1, Math.round(u.y0), Math.round(u.y1)])).toEqual([
      [0, 600, 0, 400],
      [600, 700, 0, 20],
      [600, 700, 60, 400],
      [700, 800, 0, 30],
      [700, 800, 50, 400],
    ]);
    expect(scene(new ChartData(), { showHeatmap: false }, camera(), frames()).heatmap).toBeNull();
  });
});

describe("Bubbles", () => {
  it("sit at their time and VWAP with an area proportional to notional", () => {
    const data = new ChartData();
    data.addBubbles([
      bubble(1, 9 * MINUTE_US, { notional: "1000", vwap: "99.5" }),
      bubble(2, 9 * MINUTE_US, { notional: "4000", vwap: "99.5" }),
    ]);

    const [small, big] = scene(data).bubbles;
    expect(small).toMatchObject({ x: 700, radius: BUBBLE.reference });
    expect(small?.y).toBeCloseTo(50);
    expect(big?.radius).toBeCloseTo(2 * BUBBLE.reference);
  });

  it("clamp their radius and leave the screen when outside it", () => {
    expect(bubbleRadius(1, 1_000_000)).toBe(BUBBLE.min);
    expect(bubbleRadius(1e12, 1)).toBe(BUBBLE.max);

    const data = new ChartData();
    data.addBubbles([bubble(1, 0), bubble(2, 9 * MINUTE_US, { vwap: "50" })]);
    expect(scene(data).bubbles).toEqual([]);
  });

  it("are drawn with Large Bubbles last, so they stay on top", () => {
    const data = new ChartData();
    data.addBubbles([
      bubble(1, 9 * MINUTE_US, { large: true, notional: "1000" }),
      bubble(2, 9 * MINUTE_US + 1, { notional: "5000" }),
    ]);

    expect(scene(data).bubbles.map((b) => b.bubble.first_trade_id)).toEqual([2, 1]);
  });

  it("are not laid out when the Bubbles view is off", () => {
    const data = new ChartData();
    data.addBubbles([bubble(1, 9 * MINUTE_US)]);
    expect(scene(data, { showBubbles: false }).bubbles).toEqual([]);
  });
});

describe("Footprint", () => {
  it("shows numbers only when cells are big enough to read", () => {
    expect(footprintMode(camera({ barWidth: 100, rowHeight: 14 }))).toBe("numbers");
    expect(footprintMode(camera({ barWidth: 100, rowHeight: 10 }))).toBe("profile");
    expect(footprintMode(camera({ barWidth: 40, rowHeight: 20 }))).toBe("profile");
  });

  it("places each row by its price and shades it against the largest volume on screen", () => {
    const data = new ChartData();
    data.addHistory([bar(9 * MINUTE_US, { rows: [row("99.5", "2", "1", "buy"), row("99.6", "4", "0")] })], []);

    const [column] = scene(data).columns;
    expect(column?.x).toBe(700);
    expect(column?.cells.map((c) => [Math.round(c.y), c.buyShare, c.sellShare, c.imbalance])).toEqual([
      [40, 0.5, 0.25, "buy"],
      [30, 1, 0, null],
    ]);
  });

  it("keeps columns for CVD but drops cells when the Footprint view is off", () => {
    const data = new ChartData();
    data.addHistory([bar(9 * MINUTE_US, { rows: [row("99.5", "2", "1")] })], []);

    const [column] = scene(data, { priceStyle: "candles" }).columns;
    expect(column?.cells).toEqual([]);
  });
});

describe("CVD", () => {
  const midnight = 20_765 * DAY_US;
  const cam = camera({ rightEdgeUs: midnight + 3 * MINUTE_US });

  it("breaks its line at each 00:00 UTC reset and restyles incomplete spans", () => {
    const data = new ChartData();
    data.addHistory(
      [
        bar(midnight - 2 * MINUTE_US, { cvd: "1" }),
        bar(midnight - MINUTE_US, { cvd: "0.5", cvd_complete: false }),
        bar(midnight, { cvd: "2" }),
        bar(midnight + MINUTE_US, { cvd: "1" }),
      ],
      [],
    );

    const built = scene(data, {}, cam);
    expect(built.cvd?.segments.map((s) => [s.complete, s.points.length])).toEqual([
      [true, 1],
      [false, 2],
      [true, 2],
    ]);
    expect(built.dayResets).toEqual([500]);
    expect(built.cvd?.points.every((p) => p.y >= 430 && p.y <= 580)).toBe(true);
  });

  it("is absent when its view is off", () => {
    const data = new ChartData();
    data.addHistory([bar(midnight)], []);
    expect(scene(data, { showCvd: false }, cam).cvd).toBeNull();
  });
});

describe("axes and Gaps", () => {
  it("label prices on round multiples of the Price Grouping, at least 28 px apart", () => {
    expect(niceMultiple(0.1, 0.28)).toBeCloseTo(0.5);
    expect(niceMultiple(1, 1)).toBe(1);
    expect(niceMultiple(0.1, 12)).toBeCloseTo(20);

    const ticks = scene(new ChartData()).priceTicks;
    expect(ticks.map((t) => t.label)).toEqual(
      ["96.0", "96.5", "97.0", "97.5", "98.0", "98.5", "99.0", "99.5", "100.0"],
    );
    expect(ticks[1]!.pos - ticks[0]!.pos).toBeCloseTo(-50);
  });

  it("label time every minute at 100 px per minute, and name the day at 00:00", () => {
    const midnight = 20_765 * DAY_US;
    const ticks = scene(new ChartData(), {}, camera({ rightEdgeUs: midnight + 2 * MINUTE_US })).timeTicks;

    expect(ticks.map((t) => t.label)).toEqual([
      "23:54", "23:55", "23:56", "23:57", "23:58", "23:59", "Nov 8", "00:01", "00:02",
    ]);
    expect(ticks.find((t) => t.major)?.pos).toBe(600);
  });

  it("shade open Gaps to the edge of the plot", () => {
    const data = new ChartData();
    data.addHistory([], [{ start_us: 9 * MINUTE_US, end_us: null, reason: "disconnect" }]);

    expect(scene(data).gapBands).toEqual([{ x0: 700, x1: 800, label: "No data · disconnect" }]);
  });
});
