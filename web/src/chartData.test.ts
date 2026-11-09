import { describe, expect, it } from "vitest";
import type { Bar, Gap } from "./api";
import { ChartData } from "./chartData";

const MIN = 60_000_000;

function bar(start: number, trades: number, extra: Partial<Bar> = {}): Bar {
  return {
    start_us: start,
    end_us: start + MIN,
    incomplete: false,
    buy: "1",
    sell: "1",
    delta: "0",
    total: "2",
    trades,
    rows: [],
    stacks: [],
    ...extra,
  };
}

describe("history and live merged into one series", () => {
  it("keeps bars ordered by start and lets live updates replace history", () => {
    const data = new ChartData();
    data.addHistory([bar(2 * MIN, 5), bar(0, 1)], []);
    data.applyLive({ type: "bar", bar: bar(2 * MIN, 7), closed: false });
    data.applyLive({ type: "bar", bar: bar(3 * MIN, 1), closed: false });

    expect(data.bars().map((b) => [b.start_us, b.trades])).toEqual([
      [0, 1],
      [2 * MIN, 7],
      [3 * MIN, 1],
    ]);
    expect(data.oldestStart()).toBe(0);
    expect(data.newestEnd()).toBe(4 * MIN);
  });

  it("prepending older history does not disturb newer bars", () => {
    const data = new ChartData();
    data.addHistory([bar(5 * MIN, 9)], []);
    data.addHistory([bar(MIN, 2), bar(5 * MIN, 1)], []);

    // Newer data wins: a bar already held is not overwritten by older history.
    expect(data.bars().map((b) => [b.start_us, b.trades])).toEqual([
      [MIN, 2],
      [5 * MIN, 9],
    ]);
  });

  it("a gap closing later replaces its open version", () => {
    const data = new ChartData();
    const open: Gap = { start_us: 10, end_us: null, reason: "disconnect" };
    data.addHistory([], [open]);
    data.applyLive({ type: "gap", gap: { ...open, end_us: 20 } });

    expect(data.gaps()).toEqual([{ start_us: 10, end_us: 20, reason: "disconnect" }]);
  });

  it("a bar is incomplete when any known gap overlaps it, even one learned later", () => {
    const data = new ChartData();
    data.addHistory([bar(0, 1), bar(MIN, 1)], []);
    expect(data.isIncomplete(data.bars()[0]!)).toBe(false);

    data.applyLive({ type: "gap", gap: { start_us: MIN - 1, end_us: MIN, reason: "sequence_break" } });

    expect(data.isIncomplete(data.bars()[0]!)).toBe(true);
    expect(data.isIncomplete(data.bars()[1]!)).toBe(false);
  });

  it("clear forgets everything", () => {
    const data = new ChartData();
    data.addHistory([bar(0, 1)], [{ start_us: 0, end_us: 1, reason: "x" }]);
    data.clear();
    expect(data.bars()).toEqual([]);
    expect(data.gaps()).toEqual([]);
    expect(data.oldestStart()).toBeNull();
  });
});
