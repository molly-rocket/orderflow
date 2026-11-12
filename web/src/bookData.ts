import type { Book, BookMessage, HeatmapFrame } from "./api";

export class BookData {
  #book: Book | null = null;
  #frames = new Map<number, HeatmapFrame>();
  #sorted: HeatmapFrame[] | null = null;

  clear(): void {
    this.#book = null;
    this.#frames.clear();
    this.#sorted = null;
  }

  applyBook(message: BookMessage): void {
    if (message.type === "book") {
      this.#book = message.book;
    } else if (message.type === "heatmap_history") {
      for (const frame of message.frames) this.#frames.set(frame.time_us, frame);
      this.#sorted = null;
    } else {
      this.#frames.set(message.frame.time_us, message.frame);
      this.#sorted = null;
    }
  }

  book(): Book | null {
    return this.#book;
  }

  markDisconnected(): void {
    if (!this.#book) return;
    this.#book = {
      time_us: this.#book.time_us,
      status: "disconnected",
      best_bid: null,
      best_ask: null,
      known_low: null,
      known_high: null,
      step: this.#book.step,
      rows: [],
    };
  }

  frames(): readonly HeatmapFrame[] {
    this.#sorted ??= [...this.#frames.values()].sort((a, b) => a.time_us - b.time_us);
    return this.#sorted;
  }

  prune(oldestUs: number): void {
    for (const [time] of this.#frames) {
      if (time < oldestUs) this.#frames.delete(time);
    }
    this.#sorted = null;
  }
}
