import { describe, expect, it } from "vitest";
import { BookData } from "./bookData";
import { MINUTE_US, book, frame } from "./testFactories";

describe("order book data", () => {
  it("keeps the latest book and dedupes frames by time", () => {
    const data = new BookData();
    data.applyBook({ type: "book", book: book({ status: "resyncing" }) });
    data.applyBook({ type: "book", book: book({ status: "live" }) });
    data.applyBook({ type: "heatmap_history", frames: [frame(MINUTE_US), frame(2 * MINUTE_US)] });
    data.applyBook({ type: "heatmap", frame: frame(2 * MINUTE_US, { sizes: [9] }) });
    data.applyBook({ type: "heatmap", frame: frame(3 * MINUTE_US) });

    expect(data.book()?.status).toBe("live");
    expect(data.frames().map((f) => [f.time_us, f.sizes])).toEqual([
      [MINUTE_US, [1, 2]],
      [2 * MINUTE_US, [9]],
      [3 * MINUTE_US, [1, 2]],
    ]);
  });

  it("marks the book disconnected when its socket drops, keeping the step", () => {
    const data = new BookData();
    data.applyBook({ type: "book", book: book() });

    data.markDisconnected();

    expect(data.book()).toMatchObject({ status: "disconnected", step: "0.1", rows: [], best_bid: null, known_low: null });
    expect(new BookData().book()).toBeNull();
  });

  it("prunes frames older than the oldest loaded bar and clears everything", () => {
    const data = new BookData();
    data.applyBook({ type: "heatmap_history", frames: [frame(MINUTE_US), frame(5 * MINUTE_US)] });

    data.prune(2 * MINUTE_US);
    expect(data.frames().map((f) => f.time_us)).toEqual([5 * MINUTE_US]);

    data.applyBook({ type: "book", book: book() });
    data.clear();
    expect(data.book()).toBeNull();
    expect(data.frames()).toEqual([]);
  });
});
