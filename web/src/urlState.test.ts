import { describe, expect, it } from "vitest";
import { DEFAULT_VIEW, parseView, serializeView } from "./urlState";

describe("view state in the URL", () => {
  it("restores every setting from the query string", () => {
    const view = parseView("?market=binance:perp:BTCUSDT&tf=300&grouping=25&ratio=2.5");
    expect(view).toEqual({
      market: "binance:perp:BTCUSDT",
      timeframeSecs: 300,
      grouping: 25,
      imbalanceRatio: "2.5",
    });
  });

  it("round-trips through serialize", () => {
    const view = {
      market: "hyperliquid:spot:PURR/USDC",
      timeframeSecs: 3600,
      grouping: 10,
      imbalanceRatio: "4",
    };
    expect(parseView(serializeView(view))).toEqual(view);
  });

  it("falls back to defaults for missing or invalid values", () => {
    expect(parseView("")).toEqual(DEFAULT_VIEW);
    expect(parseView("?tf=7&grouping=0&ratio=abc")).toEqual(DEFAULT_VIEW);
    expect(parseView("?tf=-60&grouping=1.5&ratio=0.5")).toEqual(DEFAULT_VIEW);
  });

  it("only accepts timeframes the gateway serves", () => {
    expect(parseView("?tf=60").timeframeSecs).toBe(60);
    expect(parseView("?tf=900").timeframeSecs).toBe(900);
    expect(parseView("?tf=30").timeframeSecs).toBe(DEFAULT_VIEW.timeframeSecs);
  });
});
