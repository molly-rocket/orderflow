import type { Bar, Book, Bubble, HeatmapFrame, Row } from "./api";
import type { Camera, Layout } from "./viewport";

export const MINUTE_US = 60_000_000;
export const DAY_US = 86_400_000_000;

export function bar(start: number, extra: Partial<Bar> = {}): Bar {
  return {
    start_us: start,
    end_us: start + MINUTE_US,
    incomplete: false,
    buy: "1",
    sell: "1",
    delta: "0",
    total: "2",
    trades: 2,
    open: "100",
    high: "100",
    low: "100",
    close: "100",
    cvd: "0",
    cvd_complete: true,
    rows: [],
    stacks: [],
    ...extra,
  };
}

export function row(price: string, buy: string, sell: string, imbalance: Row["imbalance"] = null): Row {
  return { price, buy, sell, imbalance };
}

export function bubble(firstTradeId: number, startUs: number, extra: Partial<Bubble> = {}): Bubble {
  return {
    start_us: startUs,
    end_us: startUs,
    side: "buy",
    vwap: "99.5",
    size: "1",
    notional: "1000",
    trades: 1,
    low: "99.5",
    high: "99.5",
    first_trade_id: firstTradeId,
    large: false,
    ...extra,
  };
}

export function book(extra: Partial<Book> = {}): Book {
  return {
    time_us: 9 * MINUTE_US,
    status: "live",
    best_bid: "99.5",
    best_ask: "99.6",
    known_low: "99.4",
    known_high: "99.7",
    step: "0.1",
    rows: [
      { price: "99.5", bid: "3", ask: "0" },
      { price: "99.6", bid: "0", ask: "4" },
    ],
    ...extra,
  };
}

export function frame(timeUs: number, extra: Partial<HeatmapFrame> = {}): HeatmapFrame {
  return { time_us: timeUs, status: "live", step: "0.1", low: "99.5", sizes: [1, 2], ...extra };
}

export function camera(extra: Partial<Camera> = {}): Camera {
  return {
    timeframeUs: MINUTE_US,
    barWidth: 100,
    rightEdgeUs: 10 * MINUTE_US,
    topRow: 1000,
    rowHeight: 10,
    step: 0.1,
    ...extra,
  };
}

export function layout(extra: Partial<Layout> = {}): Layout {
  return {
    width: 900,
    height: 600,
    axisWidth: 100,
    timeAxisHeight: 20,
    statsHeight: 30,
    cvdHeight: 150,
    domWidth: 0,
    ...extra,
  };
}
