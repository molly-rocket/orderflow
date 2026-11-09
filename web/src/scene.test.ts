import { describe, expect, it } from "vitest";
import type { Bar, Gap } from "./api";
import { ChartData } from "./chartData";
import { buildScene, type Viewport } from "./scene";

const MIN = 60_000_000;
const T0 = 1_794_179_700_000_000;

// A mocked gateway response: two bars a minute apart, a Gap from 30 s into the
// first bar to the start of the second, one stacked buy Imbalance.
const bars: Bar[] = [
  {
    start_us: T0,
    end_us: T0 + MIN,
    incomplete: true,
    buy: "9",
    sell: "3",
    delta: "6",
    total: "12",
    trades: 6,
    rows: [
      { price: "100", buy: "0", sell: "1", imbalance: null },
      { price: "100.1", buy: "3", sell: "1", imbalance: "buy" },
      { price: "100.2", buy: "3", sell: "1", imbalance: "buy" },
      { price: "100.3", buy: "3", sell: "0", imbalance: "buy" },
    ],
    stacks: [{ side: "buy", low: "100.1", high: "100.3" }],
  },
  {
    start_us: T0 + MIN,
    end_us: T0 + 2 * MIN,
    incomplete: false,
    buy: "0",
    sell: "2",
    delta: "-2",
    total: "2",
    trades: 1,
    rows: [{ price: "99.9", buy: "0", sell: "2", imbalance: "sell" }],
    stacks: [],
  },
];
const gaps: Gap[] = [{ start_us: T0 + MIN / 2, end_us: T0 + MIN, reason: "disconnect" }];

const viewport: Viewport = {
  width: 1000,
  height: 400,
  axisWidth: 80,
  footerHeight: 40,
  barWidth: 100,
  rowHeight: 20,
  timeframeUs: MIN,
  rightEdgeUs: T0 + 2 * MIN, // the newest bar's end sits at the axis
  // Row index = price / step. The top edge of the plot is row 1004's lower
  // edge (price 100.4), so the row starting at 100.3 is the top visible row.
  topRow: 1004,
  step: 0.1,
};

function scene() {
  const data = new ChartData();
  data.addHistory(bars, gaps);
  return buildScene(data, viewport);
}

describe("footprint scene", () => {
  it("places bars right to left from the newest, one bar width each", () => {
    const columns = scene().columns;
    expect(columns.map((c) => [c.x, c.width])).toEqual([
      [720, 100],
      [820, 100],
    ]);
  });

  it("puts each row at its price, highest at the top", () => {
    const first = scene().columns[0]!;
    expect(first.cells.map((c) => [c.price, c.y, c.height])).toEqual([
      ["100", 60, 20],
      ["100.1", 40, 20],
      ["100.2", 20, 20],
      ["100.3", 0, 20],
    ]);
  });

  it("highlights Imbalance cells and marks Stacked Imbalances", () => {
    const [first, second] = scene().columns;
    expect(first!.cells.map((c) => c.imbalance)).toEqual([null, "buy", "buy", "buy"]);
    expect(second!.cells.map((c) => c.imbalance)).toEqual(["sell"]);
    expect(first!.stacks).toEqual([{ side: "buy", y0: 0, y1: 60 }]);
  });

  it("shades a Gap as an unknown band across its time span and marks overlapped bars", () => {
    const s = scene();
    expect(s.gapBands).toEqual([{ x0: 770, x1: 820, label: "No data: disconnect" }]);
    expect(s.columns.map((c) => c.incomplete)).toEqual([true, false]);
  });

  it("an open gap runs to the right edge", () => {
    const data = new ChartData();
    data.addHistory(bars, [{ start_us: T0 + MIN + MIN / 2, end_us: null, reason: "recorder_downtime" }]);
    expect(buildScene(data, viewport).gapBands).toEqual([
      { x0: 870, x1: 920, label: "No data: recorder downtime" },
    ]);
  });

  it("culls bars and rows outside the viewport", () => {
    const data = new ChartData();
    data.addHistory(bars, gaps);
    // Plot area 100 px wide: the newest bar fills it, the older one is fully off-screen.
    const narrow = buildScene(data, { ...viewport, width: 180 });
    expect(narrow.columns.map((c) => c.x)).toEqual([0]);
    // A bar only partly on screen is still drawn.
    const partial = buildScene(data, { ...viewport, width: 230 });
    expect(partial.columns.map((c) => c.x)).toEqual([-50, 50]);
    const tall = buildScene(data, { ...viewport, topRow: 1002 });
    // Rows above the top edge are dropped.
    expect(tall.columns[0]!.cells.map((c) => c.price)).toEqual(["100", "100.1"]);
  });

  it("scales cell intensity by the largest volume on screen", () => {
    const first = scene().columns[0]!;
    expect(first.cells.map((c) => [c.buyShare, c.sellShare])).toEqual([
      [0, 1 / 3],
      [1, 1 / 3],
      [1, 1 / 3],
      [1, 0],
    ]);
  });
});
