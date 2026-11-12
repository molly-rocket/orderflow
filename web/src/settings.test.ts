import { describe, expect, it } from "vitest";
import { bookUrl } from "./api";
import {
  AUTO_GROUPING,
  DEFAULT_VIEW,
  autoGrouping,
  loadPreferences,
  parseView,
  savePreferences,
  serializeView,
} from "./settings";

describe("view state in the URL", () => {
  it("round-trips every setting", () => {
    const view = {
      market: "binance:perp:BTCUSDT",
      timeframeSecs: 300,
      grouping: 25,
      priceStyle: "line" as const,
      views: { bubbles: true, cvd: true, dom: false, heatmap: true, lastPrice: false },
      imbalanceRatio: "2.5",
      minStack: 4,
      percentile: "99.5",
      usdFloor: "250000",
      minNotional: "50000",
      side: "sell" as const,
    };
    expect(parseView(serializeView(view))).toEqual(view);
  });

  it("falls back to defaults for anything invalid", () => {
    expect(
      parseView("?tf=7&grouping=0&ratio=0.5&stack=1&pct=100&floor=-3&minusd=abc&side=up"),
    ).toEqual(DEFAULT_VIEW);
  });

  it("keeps an empty USD floor and an empty view list", () => {
    const view = parseView("?floor=&views=&price=none");
    expect(view.usdFloor).toBe("");
    expect(view.priceStyle).toBe("hidden");
    expect(view.views).toEqual({ bubbles: false, cvd: false, dom: false, heatmap: false, lastPrice: false });
  });

  it("round-trips every price style and layer through the URL", () => {
    for (const priceStyle of ["footprint", "candles", "line", "hidden"] as const) {
      const search = serializeView({ ...DEFAULT_VIEW, priceStyle });
      expect(parseView(search).priceStyle).toBe(priceStyle);
    }
    const layers = { bubbles: false, cvd: true, dom: true, heatmap: true, lastPrice: false };
    expect(parseView(serializeView({ ...DEFAULT_VIEW, views: layers })).views).toEqual(layers);
    expect(serializeView({ ...DEFAULT_VIEW, views: layers })).toContain("views=cvd%2Cdom%2Cheat");
  });

  it("legacy_views_links_keep_footprint_and_default_new_layers", () => {
    const legacy = parseView("?views=fp,bub,cvd");
    expect(legacy.priceStyle).toBe("footprint");
    expect(legacy.views).toEqual({ bubbles: true, cvd: true, dom: true, heatmap: false, lastPrice: true });
    expect(parseView("?views=bub").priceStyle).toBe("hidden");
    expect(parseView("?views=bub").views.bubbles).toBe(true);
    expect(parseView("?views=bub").views.cvd).toBe(false);
  });

  it("defaults to the footprint with bubbles, CVD, the order book and the live price on", () => {
    expect(DEFAULT_VIEW.priceStyle).toBe("footprint");
    expect(DEFAULT_VIEW.views).toEqual({ bubbles: true, cvd: true, dom: true, heatmap: false, lastPrice: true });
  });
});

describe("the order book socket URL", () => {
  const query = {
    market: "binance:perp:BTCUSDT",
    timeframeSecs: 300,
    grouping: 10,
    imbalanceRatio: "3",
    minStack: 3,
    percentile: "99",
    usdFloor: "",
    minNotional: "0",
    side: "both" as const,
  };
  const base = { protocol: "http:", host: "127.0.0.1:8080" } as Location;

  it("includes heatmap_since_us only when given", () => {
    expect(bookUrl(query, 123, base)).toBe(
      "ws://127.0.0.1:8080/api/v1/book?market=binance%3Aperp%3ABTCUSDT&grouping=10&timeframe_secs=300&heatmap_since_us=123",
    );
    expect(bookUrl(query, null, base)).toBe(
      "ws://127.0.0.1:8080/api/v1/book?market=binance%3Aperp%3ABTCUSDT&grouping=10&timeframe_secs=300",
    );
  });
});

describe("preferences in local storage", () => {
  function memory() {
    const items = new Map<string, string>();
    return {
      getItem: (key: string) => items.get(key) ?? null,
      setItem: (key: string, value: string) => void items.set(key, value),
    };
  }

  it("remember the theme and the last view", () => {
    const store = memory();
    const view = { ...DEFAULT_VIEW, timeframeSecs: 900, percentile: "95" };
    savePreferences(store, { theme: "light", view });

    expect(loadPreferences(store)).toEqual({ theme: "light", view });
  });

  it("survive missing or corrupt storage", () => {
    expect(loadPreferences(null)).toEqual({ theme: "dark", view: DEFAULT_VIEW });
    expect(loadPreferences({ getItem: () => "{not json" })).toEqual({ theme: "dark", view: DEFAULT_VIEW });
  });
});

describe("automatic Price Grouping", () => {
  it("aims for about 30 rows across a typical bar", () => {
    expect(autoGrouping([300, 280, 320])).toBe(10);
    expect(autoGrouping([3000, 2500, 3500, 4000])).toBe(100);
    expect(autoGrouping([20])).toBe(1);
    expect(autoGrouping([])).toBe(1);
    expect(autoGrouping([0, 0, 1_000_000])).toBe(1000);
  });

  it("is the default and round-trips as `auto`", () => {
    expect(DEFAULT_VIEW.grouping).toBe(AUTO_GROUPING);
    expect(serializeView(DEFAULT_VIEW)).toContain("grouping=auto");
    expect(parseView("?grouping=auto").grouping).toBe(AUTO_GROUPING);
    expect(parseView("?grouping=25").grouping).toBe(25);
  });
});
