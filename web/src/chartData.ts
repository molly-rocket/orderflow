import type { Bar, Bubble, Gap, LiveMessage } from "./api";

export class ChartData {
  #bars = new Map<number, Bar>();
  #gaps = new Map<string, Gap>();
  #bubbles = new Map<number, Bubble>();
  #sortedBars: Bar[] | null = null;
  #sortedBubbles: Bubble[] | null = null;
  #direction: boolean | null = null;

  clear(): void {
    this.#bars.clear();
    this.#gaps.clear();
    this.clearBubbles();
    this.#sortedBars = null;
    this.#direction = null;
  }

  clearBubbles(): void {
    this.#bubbles.clear();
    this.#sortedBubbles = null;
  }

  addHistory(bars: readonly Bar[], gaps: readonly Gap[]): void {
    for (const bar of bars) {
      if (!this.#bars.has(bar.start_us)) this.#bars.set(bar.start_us, bar);
    }
    for (const gap of gaps) this.#putGap(gap);
    this.#sortedBars = null;
  }

  addBubbles(bubbles: readonly Bubble[], gaps: readonly Gap[] = []): void {
    for (const bubble of bubbles) this.#bubbles.set(bubble.first_trade_id, bubble);
    for (const gap of gaps) this.#putGap(gap);
    this.#sortedBubbles = null;
  }

  applyLive(message: LiveMessage): void {
    if (message.type === "bar") {
      const newest = this.bars().at(-1);
      if (!newest || message.bar.start_us >= newest.start_us) this.#observeClose(message.bar.close);
      this.#bars.set(message.bar.start_us, message.bar);
      this.#sortedBars = null;
    } else if (message.type === "gap") {
      this.#putGap(message.gap);
    } else {
      this.addBubbles([message.bubble]);
    }
  }

  bars(): readonly Bar[] {
    this.#sortedBars ??= [...this.#bars.values()].sort((a, b) => a.start_us - b.start_us);
    return this.#sortedBars;
  }

  bubbles(): readonly Bubble[] {
    this.#sortedBubbles ??= [...this.#bubbles.values()].sort(
      (a, b) => a.start_us - b.start_us || a.first_trade_id - b.first_trade_id,
    );
    return this.#sortedBubbles;
  }

  gaps(): readonly Gap[] {
    return [...this.#gaps.values()].sort((a, b) => a.start_us - b.start_us);
  }

  oldestStart(): number | null {
    return this.bars()[0]?.start_us ?? null;
  }

  newestEnd(): number | null {
    return this.bars().at(-1)?.end_us ?? null;
  }

  lastPrice(): { price: number; up: boolean } | null {
    const bars = this.bars();
    const newest = bars.at(-1);
    if (!newest || newest.close === null) return null;
    const price = Number(newest.close);
    if (this.#direction === null) {
      const previous = bars.at(-2)?.close;
      this.#direction = previous === null || previous === undefined ? true : price >= Number(previous);
    }
    return { price, up: this.#direction };
  }

  #observeClose(close: string | null): void {
    if (close === null) return;
    const current = this.lastPrice();
    if (current && Number(close) !== current.price) this.#direction = Number(close) > current.price;
  }

  isIncomplete(bar: Bar): boolean {
    return bar.incomplete || [...this.#gaps.values()].some((g) => overlaps(g, bar.start_us, bar.end_us));
  }

  #putGap(gap: Gap): void {
    const key = `${gap.start_us}/${gap.reason}`;
    const known = this.#gaps.get(key);
    if (!known || known.end_us === null) this.#gaps.set(key, gap);
  }
}

export function overlaps(gap: Gap, start: number, end: number): boolean {
  return gap.start_us < end && (gap.end_us === null || gap.end_us > start);
}
