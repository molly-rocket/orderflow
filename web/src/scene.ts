import type { Bar, Book, BookStatus, Bubble, HeatmapFrame, ImbalanceFlag, Side } from "./api";
import type { BookData } from "./bookData";
import type { ChartData } from "./chartData";
import { clockUtc, dayLabelUtc } from "./format";
import type { PriceStyle } from "./settings";
import {
  type Camera,
  type Layout,
  type Panes,
  panes,
  priceOfY,
  timeOfX,
  xOfTime,
  yOfPrice,
} from "./viewport";

export type FootprintMode = "numbers" | "profile";

export interface SceneOptions {
  priceStyle: PriceStyle;
  showBubbles: boolean;
  showCvd: boolean;
  showDom: boolean;
  showHeatmap: boolean;
  showLastPrice: boolean;
  priceDecimals: number;
  largeThreshold: number | null;
}

export interface CandleShape {
  x: number;
  width: number;
  open: number;
  close: number;
  highY: number;
  lowY: number;
  up: boolean;
}

export interface LastPriceShape {
  y: number;
  label: string;
  up: boolean;
}

export interface LadderRowShape {
  y: number;
  height: number;
  bidShare: number;
  askShare: number;
  price: string;
  bid: string;
  ask: string;
}

export interface LadderShape {
  rows: LadderRowShape[];
  unknown: { y0: number; y1: number }[];
  status: BookStatus;
  header: string;
}

export interface HeatmapShape {
  cells: { x: number; y: number; w: number; h: number; v: number }[];
  unknown: { x0: number; x1: number; y0: number; y1: number }[];
}

export interface CellShape {
  price: string;
  buy: string;
  sell: string;
  y: number;
  height: number;
  imbalance: ImbalanceFlag;
  buyShare: number;
  sellShare: number;
}

export interface StackShape {
  side: Side;
  y0: number;
  y1: number;
}

export interface ColumnShape {
  bar: Bar;
  x: number;
  width: number;
  incomplete: boolean;
  cells: CellShape[];
  stacks: StackShape[];
}

export interface BubbleShape {
  bubble: Bubble;
  x: number;
  y: number;
  radius: number;
  alpha: number;
}

export interface CvdPointShape {
  x: number;
  y: number;
  value: number;
  complete: boolean;
  column: ColumnShape;
}

export interface CvdSegment {
  points: { x: number; y: number }[];
  complete: boolean;
}

export interface Tick {
  pos: number;
  label: string;
  major: boolean;
}

export interface CvdScale {
  min: number;
  max: number;
  top: number;
  height: number;
}

export interface CvdShape {
  points: CvdPointShape[];
  segments: CvdSegment[];
  ticks: Tick[];
  zeroY: number | null;
  scale: CvdScale;
}

export function cvdValueAt(scale: CvdScale, y: number): number {
  return scale.max - ((y - scale.top) / scale.height) * (scale.max - scale.min);
}

export interface GapBand {
  x0: number;
  x1: number;
  label: string;
}

export interface Scene {
  panes: Panes;
  mode: FootprintMode;
  columns: ColumnShape[];
  bubbles: BubbleShape[];
  cvd: CvdShape | null;
  gapBands: GapBand[];
  priceTicks: Tick[];
  timeTicks: Tick[];
  dayResets: number[];
  candles: CandleShape[];
  line: { x: number; y: number }[];
  lastPrice: LastPriceShape | null;
  ladder: LadderShape | null;
  heatmap: HeatmapShape | null;
}

export const BUBBLE = { min: 2.5, reference: 16, max: 46 };

const DAY_US = 86_400_000_000;
const TIME_TICK_SECS = [60, 120, 300, 600, 900, 1800, 3600, 7200, 10800, 14400, 21600, 43200, 86400];
const MIN_TIME_TICK_PX = 84;
const MIN_PRICE_TICK_PX = 28;

export function footprintMode(cam: Camera): FootprintMode {
  return cam.barWidth >= 68 && cam.rowHeight >= 12 ? "numbers" : "profile";
}

export function bubbleRadius(notional: number, reference: number): number {
  const scaled = BUBBLE.reference * Math.sqrt(Math.max(0, notional) / Math.max(reference, Number.EPSILON));
  return Math.min(BUBBLE.max, Math.max(BUBBLE.min, scaled));
}

export function niceStep(span: number, count: number): number {
  const raw = span / Math.max(1, count);
  const magnitude = 10 ** Math.floor(Math.log10(raw || 1));
  const normalized = raw / magnitude;
  const nice = normalized <= 1 ? 1 : normalized <= 2 ? 2 : normalized <= 5 ? 5 : 10;
  return nice * magnitude;
}

export function niceMultiple(base: number, atLeast: number): number {
  for (let magnitude = 1; magnitude < 1e12; magnitude *= 10) {
    for (const m of [1, 2, 5]) {
      if (base * m * magnitude >= atLeast - 1e-12) return base * m * magnitude;
    }
  }
  return atLeast;
}

export function buildScene(
  data: ChartData,
  cam: Camera,
  layout: Layout,
  options: SceneOptions,
  book: BookData | null = null,
): Scene {
  const p = panes(layout);
  const columns = buildColumns(data, cam, p, options.priceStyle === "footprint");
  const alpha = options.priceStyle === "footprint" ? 0.55 : 1;
  const last = options.showLastPrice ? data.lastPrice() : null;
  return {
    panes: p,
    mode: footprintMode(cam),
    columns,
    bubbles: options.showBubbles ? buildBubbles(data.bubbles(), cam, p, options.largeThreshold, alpha) : [],
    cvd: options.showCvd && p.cvd.height > 0 ? buildCvd(columns, p) : null,
    gapBands: buildGapBands(data, cam, p),
    priceTicks: buildPriceTicks(cam, p, options.priceDecimals),
    timeTicks: buildTimeTicks(cam, p),
    dayResets: buildDayResets(cam, p),
    candles: options.priceStyle === "candles" ? buildCandles(columns, cam) : [],
    line: options.priceStyle === "line" ? buildLine(columns, cam) : [],
    lastPrice: last ? { y: yOfPrice(cam, last.price), label: last.price.toFixed(options.priceDecimals), up: last.up } : null,
    ladder: options.showDom && book ? buildLadder(book.book(), cam, p, options.priceDecimals) : null,
    heatmap: options.showHeatmap && book ? buildHeatmap(book.frames(), cam, p) : null,
  };
}

function buildCandles(columns: readonly ColumnShape[], cam: Camera): CandleShape[] {
  const candles: CandleShape[] = [];
  for (const { bar, x, width } of columns) {
    if (bar.open === null || bar.close === null || bar.high === null || bar.low === null) continue;
    candles.push({
      x,
      width,
      open: yOfPrice(cam, Number(bar.open)),
      close: yOfPrice(cam, Number(bar.close)),
      highY: yOfPrice(cam, Number(bar.high)),
      lowY: yOfPrice(cam, Number(bar.low)),
      up: Number(bar.close) >= Number(bar.open),
    });
  }
  return candles;
}

function buildLine(columns: readonly ColumnShape[], cam: Camera): { x: number; y: number }[] {
  return columns
    .filter((c) => c.bar.close !== null)
    .map((c) => ({ x: c.x + c.width / 2, y: yOfPrice(cam, Number(c.bar.close)) }));
}

function clampBand(y0: number, y1: number, height: number): { y0: number; y1: number } | null {
  const top = Math.max(0, Math.min(y0, y1));
  const bottom = Math.min(height, Math.max(y0, y1));
  return bottom > top ? { y0: top, y1: bottom } : null;
}

function unknownBands(
  known: { low: number; high: number } | null,
  step: number,
  cam: Camera,
  height: number,
): { y0: number; y1: number }[] {
  if (!known) return [{ y0: 0, y1: height }];
  const bands = [clampBand(0, yOfPrice(cam, known.high + step), height), clampBand(yOfPrice(cam, known.low), height, height)];
  return bands.filter((b): b is { y0: number; y1: number } => b !== null);
}

function buildLadder(book: Book | null, cam: Camera, p: Panes, decimals: number): LadderShape {
  if (!book || book.status !== "live") {
    return { rows: [], unknown: [{ y0: 0, y1: p.main.height }], status: book?.status ?? "disconnected", header: "" };
  }
  const step = Number(book.step);
  const largest = Math.max(...book.rows.map((r) => Math.max(Number(r.bid), Number(r.ask))), 0);
  const rows: LadderRowShape[] = [];
  for (const row of book.rows) {
    const low = Number(row.price);
    const y = yOfPrice(cam, low + step);
    const height = (step / cam.step) * cam.rowHeight;
    if (y + height <= 0 || y >= p.main.height) continue;
    rows.push({
      y,
      height,
      bidShare: largest > 0 ? Number(row.bid) / largest : 0,
      askShare: largest > 0 ? Number(row.ask) / largest : 0,
      price: row.price,
      bid: row.bid,
      ask: row.ask,
    });
  }
  const known = book.known_low !== null && book.known_high !== null ? { low: Number(book.known_low), high: Number(book.known_high) } : null;
  const tick = known ? Math.min(step, Number(book.step)) : step;
  const bid = book.best_bid === null ? null : Number(book.best_bid);
  const ask = book.best_ask === null ? null : Number(book.best_ask);
  const header =
    bid !== null && ask !== null
      ? `bid ${bid.toFixed(decimals)} · ask ${ask.toFixed(decimals)} · spread ${(ask - bid).toFixed(decimals)}`
      : "";
  return { rows, unknown: unknownBands(known, tick, cam, p.main.height), status: "live", header };
}

const HEATMAP_SCALE_SAMPLES = 20_000;
const HEATMAP_LIVE_INTERVAL_US = 1_000_000;

function percentile95(values: number[]): number {
  if (values.length === 0) return 1;
  const stride = Math.max(1, Math.ceil(values.length / HEATMAP_SCALE_SAMPLES));
  const sampled: number[] = [];
  for (let i = 0; i < values.length; i += stride) sampled.push(values[i]!);
  sampled.sort((a, b) => a - b);
  return sampled[Math.min(sampled.length - 1, Math.max(0, Math.ceil(0.95 * sampled.length) - 1))] ?? 1;
}

function expectedInterval(frames: readonly HeatmapFrame[], i: number): number {
  const previous = frames[i - 1];
  const next = frames[i + 1];
  const gaps = [previous && frames[i]!.time_us - previous.time_us, next && next.time_us - frames[i]!.time_us].filter(
    (g): g is number => typeof g === "number" && g > 0,
  );
  return gaps.length === 0 ? HEATMAP_LIVE_INTERVAL_US : Math.min(...gaps);
}

function buildHeatmap(frames: readonly HeatmapFrame[], cam: Camera, p: Panes): HeatmapShape {
  const cells: HeatmapShape["cells"] = [];
  const unknown: HeatmapShape["unknown"] = [];
  const left = timeOfX(cam, p.plotWidth, 0);
  const right = timeOfX(cam, p.plotWidth, p.plotWidth);
  const spans: { frame: HeatmapFrame; x0: number; x1: number }[] = [];
  const merged = new Map<number, number>();
  for (let i = 0; i < frames.length; i++) {
    const frame = frames[i]!;
    const next = frames[i + 1];
    const cap = frame.time_us + 2 * expectedInterval(frames, i);
    const covered = next ? Math.min(next.time_us, cap) : cap;
    const until = next ? next.time_us : cap;
    if (until <= left || frame.time_us >= right) continue;
    const x0 = Math.max(0, xOfTime(cam, p.plotWidth, frame.time_us));
    const x1 = Math.min(p.plotWidth, xOfTime(cam, p.plotWidth, covered));
    const xUntil = Math.min(p.plotWidth, xOfTime(cam, p.plotWidth, until));
    if (xUntil > x1 && x1 >= 0) unknown.push({ x0: Math.max(0, x1), x1: xUntil, y0: 0, y1: p.main.height });
    if (x1 <= x0) continue;
    if (x1 - x0 < 1) {
      const column = Math.floor(x0);
      const existing = merged.get(column);
      const span = { frame, x0: column, x1: column + 1 };
      if (existing === undefined) {
        merged.set(column, spans.length);
        spans.push(span);
      } else {
        spans[existing] = span;
      }
    } else {
      spans.push({ frame, x0, x1 });
    }
  }
  const first = frames[0];
  if (!first || first.time_us > left) {
    const x1 = first ? Math.min(p.plotWidth, Math.max(0, xOfTime(cam, p.plotWidth, first.time_us))) : p.plotWidth;
    if (x1 > 0) unknown.push({ x0: 0, x1, y0: 0, y1: p.main.height });
  }
  const sizes = spans.flatMap(({ frame }) => frame.sizes.filter((s) => s > 0));
  const reference = percentile95(sizes);
  for (const { frame, x0, x1 } of spans) {
    const step = Number(frame.step);
    const low = frame.low === null ? null : Number(frame.low);
    const known = frame.status === "live" && low !== null ? { low, high: low + step * (frame.sizes.length - 1) } : null;
    for (const band of unknownBands(known, step, cam, p.main.height)) unknown.push({ x0, x1, ...band });
    if (!known) continue;
    const h = (step / cam.step) * cam.rowHeight;
    frame.sizes.forEach((size, i) => {
      if (size <= 0) return;
      const y = yOfPrice(cam, known.low + step * (i + 1));
      if (y + h <= 0 || y >= p.main.height) return;
      cells.push({ x: x0, y, w: x1 - x0, h, v: Math.min(1, Math.sqrt(size / reference)) });
    });
  }
  return { cells, unknown };
}

function buildColumns(data: ChartData, cam: Camera, p: Panes, withCells: boolean): ColumnShape[] {
  const columns: ColumnShape[] = [];
  let maxVolume = 0;
  for (const bar of data.bars()) {
    const x = xOfTime(cam, p.plotWidth, bar.start_us);
    if (x + cam.barWidth <= 0 || x >= p.plotWidth) continue;
    const cells: CellShape[] = [];
    if (withCells) {
      for (const row of bar.rows) {
        const low = Number(row.price);
        const y = yOfPrice(cam, low + cam.step);
        if (y + cam.rowHeight <= 0 || y >= p.main.height) continue;
        maxVolume = Math.max(maxVolume, Number(row.buy), Number(row.sell));
        cells.push({
          price: row.price,
          buy: row.buy,
          sell: row.sell,
          y,
          height: cam.rowHeight,
          imbalance: row.imbalance,
          buyShare: 0,
          sellShare: 0,
        });
      }
    }
    columns.push({
      bar,
      x,
      width: cam.barWidth,
      incomplete: data.isIncomplete(bar),
      cells,
      stacks: withCells
        ? bar.stacks.map((s) => ({
            side: s.side,
            y0: yOfPrice(cam, Number(s.high) + cam.step),
            y1: yOfPrice(cam, Number(s.low)),
          }))
        : [],
    });
  }
  if (maxVolume > 0) {
    for (const cell of columns.flatMap((c) => c.cells)) {
      cell.buyShare = Number(cell.buy) / maxVolume;
      cell.sellShare = Number(cell.sell) / maxVolume;
    }
  }
  return columns;
}

function buildBubbles(
  bubbles: readonly Bubble[],
  cam: Camera,
  p: Panes,
  largeThreshold: number | null,
  alpha: number,
): BubbleShape[] {
  const placed: BubbleShape[] = [];
  let largest = 0;
  for (const bubble of bubbles) largest = Math.max(largest, Number(bubble.notional));
  const reference = largeThreshold ?? largest;
  for (const bubble of bubbles) {
    const radius = bubbleRadius(Number(bubble.notional), reference);
    const x = xOfTime(cam, p.plotWidth, (bubble.start_us + bubble.end_us) / 2);
    const y = yOfPrice(cam, Number(bubble.vwap));
    if (x + radius < 0 || x - radius > p.plotWidth || y + radius < 0 || y - radius > p.main.height) continue;
    placed.push({ bubble, x, y, radius, alpha });
  }
  return placed.sort((a, b) => Number(a.bubble.large) - Number(b.bubble.large) || a.radius - b.radius);
}

function dayOf(us: number): number {
  return Math.floor(us / DAY_US);
}

function buildCvd(columns: readonly ColumnShape[], p: Panes): CvdShape | null {
  const visible = columns.filter((c) => c.x + c.width / 2 >= 0 && c.x + c.width / 2 <= p.plotWidth);
  if (visible.length === 0) return null;
  const values = visible.map((c) => Number(c.bar.cvd));
  let min = Math.min(...values);
  let max = Math.max(...values);
  if (max - min < 1e-9) {
    min -= 1;
    max += 1;
  }
  const pad = (max - min) * 0.1;
  min -= pad;
  max += pad;
  const inner = { top: p.cvd.top + 6, height: Math.max(1, p.cvd.height - 12) };
  const y = (value: number) => inner.top + ((max - value) / (max - min)) * inner.height;
  const points: CvdPointShape[] = visible.map((column) => ({
    x: column.x + column.width / 2,
    y: y(Number(column.bar.cvd)),
    value: Number(column.bar.cvd),
    complete: column.bar.cvd_complete,
    column,
  }));
  const segments: CvdSegment[] = [];
  let previous: CvdPointShape | null = null;
  for (const point of points) {
    const sameDay = previous !== null && dayOf(previous.column.bar.start_us) === dayOf(point.column.bar.start_us);
    const current = segments.at(-1);
    if (current && sameDay && current.complete === point.complete) {
      current.points.push({ x: point.x, y: point.y });
    } else {
      const start = sameDay && previous ? [{ x: previous.x, y: previous.y }] : [];
      segments.push({ points: [...start, { x: point.x, y: point.y }], complete: point.complete });
    }
    previous = point;
  }
  const step = niceStep(max - min, 3);
  const ticks: Tick[] = [];
  for (let v = Math.ceil(min / step) * step; v <= max; v += step) {
    ticks.push({ pos: y(v), label: formatSigned(v, step), major: Math.abs(v) < step / 2 });
  }
  return {
    points,
    segments,
    ticks,
    zeroY: min <= 0 && max >= 0 ? y(0) : null,
    scale: { min, max, top: inner.top, height: inner.height },
  };
}

function formatSigned(value: number, step: number): string {
  const decimals = step >= 1 ? 0 : Math.min(6, Math.ceil(-Math.log10(step)));
  const text = Math.abs(value) < step / 2 ? "0" : value.toFixed(decimals);
  return value > step / 2 ? `+${text}` : text;
}

function buildGapBands(data: ChartData, cam: Camera, p: Panes): GapBand[] {
  const bands: GapBand[] = [];
  for (const gap of data.gaps()) {
    const x0 = Math.max(0, xOfTime(cam, p.plotWidth, gap.start_us));
    const x1 = Math.min(p.plotWidth, gap.end_us === null ? p.plotWidth : xOfTime(cam, p.plotWidth, gap.end_us));
    if (x1 > x0) bands.push({ x0, x1, label: `No data · ${gap.reason.replaceAll("_", " ")}` });
  }
  return bands;
}

function buildPriceTicks(cam: Camera, p: Panes, decimals: number): Tick[] {
  if (p.main.height <= 0) return [];
  const step = niceMultiple(cam.step, (MIN_PRICE_TICK_PX / cam.rowHeight) * cam.step);
  const low = priceOfY(cam, p.main.height);
  const high = priceOfY(cam, 0);
  const ticks: Tick[] = [];
  const first = Math.ceil(low / step - 1e-9);
  const last = Math.floor(high / step + 1e-9);
  for (let k = first; k <= last && ticks.length < 400; k++) {
    const price = k * step;
    ticks.push({ pos: yOfPrice(cam, price), label: price.toFixed(decimals), major: false });
  }
  return ticks;
}

function buildTimeTicks(cam: Camera, p: Panes): Tick[] {
  const pxPerSec = cam.barWidth / (cam.timeframeUs / 1e6);
  const intervalSecs = TIME_TICK_SECS.find((s) => s * pxPerSec >= MIN_TIME_TICK_PX) ?? 86400;
  const intervalUs = intervalSecs * 1e6;
  const left = timeOfX(cam, p.plotWidth, 0);
  const right = timeOfX(cam, p.plotWidth, p.plotWidth);
  const ticks: Tick[] = [];
  for (let t = Math.ceil(left / intervalUs) * intervalUs; t <= right && ticks.length < 200; t += intervalUs) {
    const major = t % DAY_US === 0;
    ticks.push({ pos: xOfTime(cam, p.plotWidth, t), label: major ? dayLabelUtc(t) : clockUtc(t), major });
  }
  return ticks;
}

function buildDayResets(cam: Camera, p: Panes): number[] {
  const left = timeOfX(cam, p.plotWidth, 0);
  const right = timeOfX(cam, p.plotWidth, p.plotWidth);
  const resets: number[] = [];
  for (let day = Math.ceil(left / DAY_US) * DAY_US; day <= right && resets.length < 100; day += DAY_US) {
    resets.push(xOfTime(cam, p.plotWidth, day));
  }
  return resets;
}
