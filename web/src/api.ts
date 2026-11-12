export interface Market {
  id: string;
  venue: string;
  market_type: string;
  native_name: string;
  tick_size: string;
  recording_horizon_us: number | null;
  burst_grouped: boolean;
}

export type ImbalanceFlag = "buy" | "sell" | "both" | null;

export type Side = "buy" | "sell";

export interface Row {
  price: string;
  buy: string;
  sell: string;
  imbalance: ImbalanceFlag;
}

export interface StackedImbalance {
  side: Side;
  low: string;
  high: string;
}

export interface Bar {
  start_us: number;
  end_us: number;
  incomplete: boolean;
  buy: string;
  sell: string;
  delta: string;
  total: string;
  trades: number;
  open: string | null;
  high: string | null;
  low: string | null;
  close: string | null;
  cvd: string;
  cvd_complete: boolean;
  rows: Row[];
  stacks: StackedImbalance[];
}

export interface Gap {
  start_us: number;
  end_us: number | null;
  reason: string;
}

export interface Bubble {
  start_us: number;
  end_us: number;
  side: Side;
  vwap: string;
  size: string;
  notional: string;
  trades: number;
  low: string;
  high: string;
  first_trade_id: number;
  large: boolean;
}

export interface FootprintResponse {
  market: string;
  timeframe_secs: number;
  grouping: { tick: string; multiple: number; step: string };
  imbalance: { ratio: string; min_stack: number };
  bars: Bar[];
  gaps: Gap[];
}

export interface LargeBubbleInfo {
  percentile: string;
  usd_floor: string | null;
  threshold: string | null;
  window_start_us: number;
  window_end_us: number;
}

export interface BubblesResponse {
  market: string;
  burst_grouped: boolean;
  large: LargeBubbleInfo;
  bubbles: Bubble[];
  truncated: boolean;
  gaps: Gap[];
}

export type LiveMessage =
  | { type: "bar"; bar: Bar; closed: boolean }
  | { type: "gap"; gap: Gap }
  | { type: "bubble"; bubble: Bubble };

export type BookStatus = "live" | "resyncing" | "disconnected";

export interface BookLevelRow {
  price: string;
  bid: string;
  ask: string;
}

export interface Book {
  time_us: number;
  status: BookStatus;
  best_bid: string | null;
  best_ask: string | null;
  known_low: string | null;
  known_high: string | null;
  step: string;
  rows: BookLevelRow[];
}

export interface HeatmapFrame {
  time_us: number;
  status: BookStatus;
  step: string;
  low: string | null;
  sizes: number[];
}

export type BookMessage =
  | { type: "book"; book: Book }
  | { type: "heatmap_history"; frames: HeatmapFrame[] }
  | { type: "heatmap"; frame: HeatmapFrame };

export interface ChartQuery {
  market: string;
  timeframeSecs: number;
  grouping: number;
  imbalanceRatio: string;
  minStack: number;
  percentile: string;
  usdFloor: string;
  minNotional: string;
  side: "both" | Side;
}

export class ApiError extends Error {
  constructor(
    readonly status: number,
    message: string,
  ) {
    super(message);
  }
}

async function getJson<T>(path: string, signal?: AbortSignal): Promise<T> {
  const response = await fetch(path, signal ? { signal } : {});
  const body: unknown = await response.json();
  if (!response.ok) {
    const error = (body as { error?: string }).error ?? response.statusText;
    throw new ApiError(response.status, error);
  }
  return body as T;
}

function barParams(q: ChartQuery): URLSearchParams {
  return new URLSearchParams({
    market: q.market,
    timeframe_secs: String(q.timeframeSecs),
    grouping: String(q.grouping),
    imbalance_ratio: q.imbalanceRatio,
    min_stack: String(q.minStack),
  });
}

function bubbleParams(q: ChartQuery, into: URLSearchParams): URLSearchParams {
  into.set("percentile", q.percentile);
  if (q.usdFloor !== "") into.set("usd_floor", q.usdFloor);
  if (q.minNotional !== "") into.set("min_notional", q.minNotional);
  into.set("side", q.side);
  return into;
}

export function fetchMarkets(signal?: AbortSignal): Promise<Market[]> {
  return getJson("/api/v1/markets", signal);
}

export function fetchFootprint(
  q: ChartQuery,
  fromUs: number,
  toUs: number,
  signal?: AbortSignal,
): Promise<FootprintResponse> {
  const params = barParams(q);
  params.set("from_us", String(fromUs));
  params.set("to_us", String(toUs));
  return getJson(`/api/v1/footprint?${params}`, signal);
}

export function fetchBubbles(
  q: ChartQuery,
  fromUs: number,
  toUs: number,
  signal?: AbortSignal,
): Promise<BubblesResponse> {
  const params = bubbleParams(q, new URLSearchParams({ market: q.market }));
  params.set("from_us", String(fromUs));
  params.set("to_us", String(toUs));
  return getJson(`/api/v1/bubbles?${params}`, signal);
}

export function bookUrl(q: ChartQuery, heatmapSinceUs: number | null, base: Location = location): string {
  const params = new URLSearchParams({
    market: q.market,
    grouping: String(q.grouping),
    timeframe_secs: String(q.timeframeSecs),
  });
  if (heatmapSinceUs !== null) params.set("heatmap_since_us", String(heatmapSinceUs));
  const scheme = base.protocol === "https:" ? "wss:" : "ws:";
  return `${scheme}//${base.host}/api/v1/book?${params}`;
}

export function liveUrl(q: ChartQuery, sinceUs: number, base: Location = location): string {
  const params = bubbleParams(q, barParams(q));
  params.set("since_us", String(sinceUs));
  const scheme = base.protocol === "https:" ? "wss:" : "ws:";
  return `${scheme}//${base.host}/api/v1/live?${params}`;
}
