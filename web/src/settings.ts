import type { Side } from "./api";

export type Theme = "dark" | "light";

export type BubbleSide = "both" | Side;

export type PriceStyle = "footprint" | "candles" | "line" | "hidden";

export const PRICE_STYLES: readonly { value: PriceStyle; label: string }[] = [
  { value: "footprint", label: "Footprint" },
  { value: "candles", label: "Candles" },
  { value: "line", label: "Line" },
  { value: "hidden", label: "Hidden" },
];

export interface Views {
  bubbles: boolean;
  cvd: boolean;
  dom: boolean;
  heatmap: boolean;
  lastPrice: boolean;
}

export interface ViewState {
  market: string | null;
  timeframeSecs: number;
  grouping: number;
  priceStyle: PriceStyle;
  views: Views;
  imbalanceRatio: string;
  minStack: number;
  percentile: string;
  usdFloor: string;
  minNotional: string;
  side: BubbleSide;
}

export const TIMEFRAMES: readonly { secs: number; label: string }[] = [
  { secs: 60, label: "1m" },
  { secs: 300, label: "5m" },
  { secs: 900, label: "15m" },
  { secs: 1800, label: "30m" },
  { secs: 3600, label: "1h" },
  { secs: 14400, label: "4h" },
];

export const GROUPINGS: readonly number[] = [1, 2, 5, 10, 25, 50, 100, 250, 500, 1000];

export const AUTO_GROUPING = 0;

const TARGET_ROWS_PER_BAR = 30;

export function autoGrouping(barRangesInTicks: readonly number[]): number {
  const sorted = [...barRangesInTicks].filter((r) => r > 0).sort((a, b) => a - b);
  const median = sorted[Math.floor(sorted.length / 2)];
  if (median === undefined) return GROUPINGS[0] ?? 1;
  let best = GROUPINGS[0] ?? 1;
  for (const multiple of GROUPINGS) {
    const distance = Math.abs(Math.log(median / multiple / TARGET_ROWS_PER_BAR));
    if (distance < Math.abs(Math.log(median / best / TARGET_ROWS_PER_BAR))) best = multiple;
  }
  return best;
}

export const IMBALANCE_RATIOS: readonly { value: string; label: string }[] = [
  { value: "1.5", label: "150%" },
  { value: "2", label: "200%" },
  { value: "2.5", label: "250%" },
  { value: "3", label: "300%" },
  { value: "4", label: "400%" },
  { value: "5", label: "500%" },
];

export const PERCENTILES: readonly { value: string; label: string }[] = [
  { value: "90", label: "Top 10%" },
  { value: "95", label: "Top 5%" },
  { value: "99", label: "Top 1%" },
  { value: "99.5", label: "Top 0.5%" },
  { value: "99.9", label: "Top 0.1%" },
];

export const DEFAULT_VIEW: ViewState = {
  market: null,
  timeframeSecs: 60,
  grouping: AUTO_GROUPING,
  priceStyle: "footprint",
  views: { bubbles: true, cvd: true, dom: true, heatmap: false, lastPrice: true },
  imbalanceRatio: "3",
  minStack: 3,
  percentile: "99",
  usdFloor: "",
  minNotional: "10000",
  side: "both",
};

const DECIMAL = /^\d+(\.\d+)?$/;

function decimalOr(raw: string | null | undefined, fallback: string, valid: (n: number) => boolean): string {
  return raw !== null && raw !== undefined && DECIMAL.test(raw) && valid(Number(raw)) ? raw : fallback;
}

const PRICE_KEYS: readonly [PriceStyle, string][] = [
  ["footprint", "fp"],
  ["candles", "candles"],
  ["line", "line"],
  ["hidden", "none"],
];

const LAYER_KEYS: readonly [keyof Views, string][] = [
  ["bubbles", "bub"],
  ["cvd", "cvd"],
  ["dom", "dom"],
  ["heatmap", "heat"],
  ["lastPrice", "last"],
];

function parsePriceStyle(raw: string | null, legacyViews: string | null, fallback: PriceStyle): PriceStyle {
  const named = PRICE_KEYS.find(([, key]) => key === raw)?.[0];
  if (named) return named;
  if (raw === null && legacyViews !== null) return legacyViews.split(",").includes("fp") ? "footprint" : "hidden";
  return fallback;
}

function parseViews(raw: string | null, legacy: boolean, fallback: Views): Views {
  if (raw === null) return fallback;
  const names = new Set(raw.split(",").filter(Boolean));
  const views = { ...fallback };
  for (const [layer, key] of LAYER_KEYS) {
    if (legacy && (layer === "dom" || layer === "heatmap" || layer === "lastPrice")) continue;
    views[layer] = names.has(key);
  }
  return views;
}

function serializeViews(views: Views): string {
  return LAYER_KEYS.filter(([layer]) => views[layer])
    .map(([, key]) => key)
    .join(",");
}

export function parseView(search: string, defaults: ViewState = DEFAULT_VIEW): ViewState {
  const params = new URLSearchParams(search);
  const tf = Number(params.get("tf"));
  const rawGrouping = params.get("grouping");
  const grouping = rawGrouping === "auto" ? AUTO_GROUPING : Number(rawGrouping);
  const stack = Number(params.get("stack"));
  const side = params.get("side");
  const floor = params.get("floor");
  const rawPrice = params.get("price");
  const rawViews = params.get("views");
  return {
    market: params.get("market") || defaults.market,
    timeframeSecs: TIMEFRAMES.some((t) => t.secs === tf) ? tf : defaults.timeframeSecs,
    grouping: rawGrouping !== null && Number.isInteger(grouping) && grouping >= 0 ? grouping : defaults.grouping,
    priceStyle: parsePriceStyle(rawPrice, rawViews, defaults.priceStyle),
    views: parseViews(rawViews, rawPrice === null, defaults.views),
    imbalanceRatio: decimalOr(params.get("ratio"), defaults.imbalanceRatio, (n) => n >= 1),
    minStack: Number.isInteger(stack) && stack >= 2 && stack <= 20 ? stack : defaults.minStack,
    percentile: decimalOr(params.get("pct"), defaults.percentile, (n) => n > 0 && n < 100),
    usdFloor: floor === "" ? "" : decimalOr(floor, defaults.usdFloor, () => true),
    minNotional: decimalOr(params.get("minusd"), defaults.minNotional, () => true),
    side: side === "buy" || side === "sell" || side === "both" ? side : defaults.side,
  };
}

export function serializeView(view: ViewState): string {
  const params = new URLSearchParams();
  if (view.market) params.set("market", view.market);
  params.set("tf", String(view.timeframeSecs));
  params.set("grouping", view.grouping === AUTO_GROUPING ? "auto" : String(view.grouping));
  params.set("price", PRICE_KEYS.find(([style]) => style === view.priceStyle)?.[1] ?? "fp");
  params.set("views", serializeViews(view.views));
  params.set("ratio", view.imbalanceRatio);
  params.set("stack", String(view.minStack));
  params.set("pct", view.percentile);
  params.set("floor", view.usdFloor);
  params.set("minusd", view.minNotional);
  params.set("side", view.side);
  return `?${params}`;
}

export interface Preferences {
  theme: Theme;
  view: ViewState;
}

const PREFERENCES_KEY = "orderflow.preferences.v1";

export function loadPreferences(storage: Pick<Storage, "getItem"> | null): Preferences {
  const fallback: Preferences = { theme: "dark", view: DEFAULT_VIEW };
  try {
    const raw = storage?.getItem(PREFERENCES_KEY);
    if (!raw) return fallback;
    const stored = JSON.parse(raw) as { theme?: unknown; search?: unknown };
    return {
      theme: stored.theme === "light" ? "light" : "dark",
      view: typeof stored.search === "string" ? parseView(stored.search) : DEFAULT_VIEW,
    };
  } catch {
    return fallback;
  }
}

export function savePreferences(storage: Pick<Storage, "setItem"> | null, preferences: Preferences): void {
  try {
    storage?.setItem(
      PREFERENCES_KEY,
      JSON.stringify({ theme: preferences.theme, search: serializeView(preferences.view) }),
    );
  } catch {
    return;
  }
}
