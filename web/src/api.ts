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

export interface Row {
  /** Lowest price of the row. */
  price: string;
  buy: string;
  sell: string;
  imbalance: ImbalanceFlag;
}

export interface StackedImbalance {
  side: "buy" | "sell";
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
  rows: Row[];
  stacks: StackedImbalance[];
}

export interface Gap {
  start_us: number;
  end_us: number | null;
  reason: string;
}

export interface FootprintResponse {
  market: string;
  timeframe_secs: number;
  grouping: { tick: string; multiple: number; step: string };
  imbalance: { ratio: string; min_stack: number };
  bars: Bar[];
  gaps: Gap[];
}

export type LiveMessage =
  | { type: "bar"; bar: Bar; closed: boolean }
  | { type: "gap"; gap: Gap };

export interface BarQuery {
  market: string;
  timeframeSecs: number;
  grouping: number;
  imbalanceRatio: string;
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

function barParams(q: BarQuery): URLSearchParams {
  return new URLSearchParams({
    market: q.market,
    timeframe_secs: String(q.timeframeSecs),
    grouping: String(q.grouping),
    imbalance_ratio: q.imbalanceRatio,
  });
}

export function fetchMarkets(signal?: AbortSignal): Promise<Market[]> {
  return getJson("/api/v1/markets", signal);
}

export function fetchFootprint(
  q: BarQuery,
  fromUs: number,
  toUs: number,
  signal?: AbortSignal,
): Promise<FootprintResponse> {
  const params = barParams(q);
  params.set("from_us", String(fromUs));
  params.set("to_us", String(toUs));
  return getJson(`/api/v1/footprint?${params}`, signal);
}

/** WebSocket URL for live bars from the bar containing `sinceUs`. */
export function liveUrl(q: BarQuery, sinceUs: number, base: Location = location): string {
  const params = barParams(q);
  params.set("since_us", String(sinceUs));
  const scheme = base.protocol === "https:" ? "wss:" : "ws:";
  return `${scheme}//${base.host}/api/v1/live?${params}`;
}
