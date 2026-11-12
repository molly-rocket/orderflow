import { describe, expect, it } from "vitest";
import type { Gap } from "./api";
import { ChartData } from "./chartData";
import { MINUTE_US as MIN, bar, bubble } from "./testFactories";

describe("history and live merged into one series", () => {
  it("keeps bars ordered by start and lets live updates replace history", () => {
    const data = new ChartData();
    data.addHistory([bar(2 * MIN, { trades: 5 }), bar(0, { trades: 1 })], []);
    data.applyLive({ type: "bar", bar: bar(2 * MIN, { trades: 7 }), closed: false });
    data.applyLive({ type: "bar", bar: bar(3 * MIN, { trades: 1 }), closed: false });

    expect(data.bars().map((b) => [b.start_us, b.trades])).toEqual([
      [0, 1],
      [2 * MIN, 7],
      [3 * MIN, 1],
    ]);
    expect(data.oldestStart()).toBe(0);
    expect(data.newestEnd()).toBe(4 * MIN);
  });

  it("does not let older history overwrite a bar already held", () => {
    const data = new ChartData();
    data.addHistory([bar(5 * MIN, { trades: 9 })], []);
    data.addHistory([bar(MIN, { trades: 2 }), bar(5 * MIN, { trades: 1 })], []);

    expect(data.bars().map((b) => [b.start_us, b.trades])).toEqual([
      [MIN, 2],
      [5 * MIN, 9],
    ]);
  });

  it("replaces an open Gap with its closed version, never the reverse", () => {
    const data = new ChartData();
    const open: Gap = { start_us: 10, end_us: null, reason: "disconnect" };
    data.addHistory([], [open]);
    data.applyLive({ type: "gap", gap: { ...open, end_us: 20 } });
    data.addHistory([], [open]);

    expect(data.gaps()).toEqual([{ start_us: 10, end_us: 20, reason: "disconnect" }]);
  });

  it("marks a bar incomplete when any known Gap overlaps it, even one learned later", () => {
    const data = new ChartData();
    data.addHistory([bar(0), bar(MIN)], []);
    expect(data.isIncomplete(data.bars()[0]!)).toBe(false);

    data.applyLive({ type: "gap", gap: { start_us: MIN - 1, end_us: MIN, reason: "sequence_break" } });

    expect(data.isIncomplete(data.bars()[0]!)).toBe(true);
    expect(data.isIncomplete(data.bars()[1]!)).toBe(false);
  });

  it("keeps one Bubble per first Trade id, ordered by time, from history and live alike", () => {
    const data = new ChartData();
    data.addBubbles([bubble(7, 30), bubble(3, 10)]);
    data.applyLive({ type: "bubble", bubble: bubble(7, 30, { large: true }) });
    data.applyLive({ type: "bubble", bubble: bubble(9, 20) });

    expect(data.bubbles().map((b) => [b.first_trade_id, b.large])).toEqual([
      [3, false],
      [9, false],
      [7, true],
    ]);
  });

  it("clears Bubbles alone for a new filter, or everything for a new view", () => {
    const data = new ChartData();
    data.addHistory([bar(0)], [{ start_us: 0, end_us: 1, reason: "x" }]);
    data.addBubbles([bubble(1, 0)]);

    data.clearBubbles();
    expect(data.bubbles()).toEqual([]);
    expect(data.bars()).toHaveLength(1);

    data.clear();
    expect(data.bars()).toEqual([]);
    expect(data.gaps()).toEqual([]);
    expect(data.oldestStart()).toBeNull();
  });
});
