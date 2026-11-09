// One Market's bars and Gaps for the current view, merged from HTTP history
// pages and live WebSocket updates.

import type { Bar, Gap, LiveMessage } from "./api";

export class ChartData {
  /** Bars by start time; kept sorted on read. */
  #bars = new Map<number, Bar>();
  /** Gaps by `start_us/reason`: an open Gap and its later close share a key. */
  #gaps = new Map<string, Gap>();
  #sorted: Bar[] | null = null;

  clear(): void {
    this.#bars.clear();
    this.#gaps.clear();
    this.#sorted = null;
  }

  /** Adds a history page. Bars already held (e.g. updated live) are newer and win. */
  addHistory(bars: readonly Bar[], gaps: readonly Gap[]): void {
    for (const bar of bars) {
      if (!this.#bars.has(bar.start_us)) this.#bars.set(bar.start_us, bar);
    }
    for (const gap of gaps) this.#putGap(gap);
    this.#sorted = null;
  }

  applyLive(message: LiveMessage): void {
    if (message.type === "bar") {
      this.#bars.set(message.bar.start_us, message.bar);
      this.#sorted = null;
    } else {
      this.#putGap(message.gap);
    }
  }

  /** Bars ascending by start time. */
  bars(): readonly Bar[] {
    this.#sorted ??= [...this.#bars.values()].sort((a, b) => a.start_us - b.start_us);
    return this.#sorted;
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

  /** Incomplete per the gateway, or overlapped by a Gap learned since. */
  isIncomplete(bar: Bar): boolean {
    return bar.incomplete || [...this.#gaps.values()].some((g) => overlaps(g, bar.start_us, bar.end_us));
  }

  #putGap(gap: Gap): void {
    const key = `${gap.start_us}/${gap.reason}`;
    const known = this.#gaps.get(key);
    // A closed version is newer than an open one; never reopen.
    if (!known || known.end_us === null) this.#gaps.set(key, gap);
  }
}

/** Whether `gap` shares any instant with `[start, end)`. */
export function overlaps(gap: Gap, start: number, end: number): boolean {
  return gap.start_us < end && (gap.end_us === null || gap.end_us > start);
}
