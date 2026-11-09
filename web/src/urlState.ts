// The chart's view settings, kept in the URL so a view can be bookmarked and shared.

export interface View {
  /** Market id, `venue:type:native-name`; null = the first Market the gateway serves. */
  market: string | null;
  timeframeSecs: number;
  /** Price Grouping as a multiple of the Market's tick size. */
  grouping: number;
  /** Imbalance ratio as a multiplier (3 = 300%). */
  imbalanceRatio: string;
}

/** Timeframes offered; all divide the Trading Day and are whole minutes (gateway rule). */
export const TIMEFRAMES: readonly { secs: number; label: string }[] = [
  { secs: 60, label: "1m" },
  { secs: 300, label: "5m" },
  { secs: 900, label: "15m" },
  { secs: 3600, label: "1h" },
  { secs: 14400, label: "4h" },
];

/** Price Grouping multiples offered. */
export const GROUPINGS: readonly number[] = [1, 2, 5, 10, 25, 50, 100, 250, 500, 1000];

export const DEFAULT_VIEW: View = {
  market: null,
  timeframeSecs: 60,
  grouping: 1,
  imbalanceRatio: "3",
};

export function parseView(search: string): View {
  const params = new URLSearchParams(search);
  const tf = Number(params.get("tf"));
  const grouping = Number(params.get("grouping"));
  const ratio = params.get("ratio");
  return {
    market: params.get("market") || DEFAULT_VIEW.market,
    timeframeSecs: TIMEFRAMES.some((t) => t.secs === tf) ? tf : DEFAULT_VIEW.timeframeSecs,
    grouping: Number.isInteger(grouping) && grouping >= 1 ? grouping : DEFAULT_VIEW.grouping,
    imbalanceRatio:
      ratio !== null && /^\d+(\.\d+)?$/.test(ratio) && Number(ratio) >= 1
        ? ratio
        : DEFAULT_VIEW.imbalanceRatio,
  };
}

export function serializeView(view: View): string {
  const params = new URLSearchParams();
  if (view.market) params.set("market", view.market);
  params.set("tf", String(view.timeframeSecs));
  params.set("grouping", String(view.grouping));
  params.set("ratio", view.imbalanceRatio);
  return `?${params}`;
}
