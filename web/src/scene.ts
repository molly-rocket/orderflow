// Turns chart data and a viewport into positioned shapes. Pure: no DOM, no
// canvas. The painter (render.ts) only draws what this decides, so layout,
// culling, Gap shading and Imbalance highlighting are all tested here.

import type { ImbalanceFlag } from "./api";
import type { ChartData } from "./chartData";

export interface Viewport {
  /** CSS pixels. */
  width: number;
  height: number;
  /** Price axis on the right. */
  axisWidth: number;
  /** Per-bar totals under the plot. */
  footerHeight: number;
  barWidth: number;
  rowHeight: number;
  timeframeUs: number;
  /** Time at the plot's right edge. */
  rightEdgeUs: number;
  /** Row index (price / step) whose lower edge is at y = 0; fractional while panning. */
  topRow: number;
  /** Price Grouping step as a number (positions only; text stays exact). */
  step: number;
}

export interface Cell {
  price: string;
  buy: string;
  sell: string;
  y: number;
  height: number;
  imbalance: ImbalanceFlag;
  /** Volume relative to the largest single-side volume on screen, 0..1. */
  buyShare: number;
  sellShare: number;
}

export interface Column {
  x: number;
  width: number;
  startUs: number;
  incomplete: boolean;
  cells: Cell[];
  stacks: { side: "buy" | "sell"; y0: number; y1: number }[];
  delta: string;
  total: string;
  trades: number;
}

export interface GapBand {
  x0: number;
  x1: number;
  label: string;
}

export interface Scene {
  columns: Column[];
  gapBands: GapBand[];
  plotWidth: number;
  plotHeight: number;
}

export function rowIndex(price: string, step: number): number {
  return Math.round(Number(price) / step);
}

export function buildScene(data: ChartData, vp: Viewport): Scene {
  const plotWidth = vp.width - vp.axisWidth;
  const plotHeight = vp.height - vp.footerHeight;
  const x = (us: number) => plotWidth - ((vp.rightEdgeUs - us) / vp.timeframeUs) * vp.barWidth;
  const rowTop = (row: number) => (vp.topRow - row - 1) * vp.rowHeight;

  const columns: Column[] = [];
  let maxVolume = 0;
  for (const bar of data.bars()) {
    const left = x(bar.start_us);
    if (left + vp.barWidth <= 0 || left >= plotWidth) continue;
    const cells: Cell[] = [];
    for (const row of bar.rows) {
      const y = rowTop(rowIndex(row.price, vp.step));
      if (y + vp.rowHeight <= 0 || y >= plotHeight) continue;
      maxVolume = Math.max(maxVolume, Number(row.buy), Number(row.sell));
      cells.push({
        price: row.price,
        buy: row.buy,
        sell: row.sell,
        y,
        height: vp.rowHeight,
        imbalance: row.imbalance,
        buyShare: 0,
        sellShare: 0,
      });
    }
    columns.push({
      x: left,
      width: vp.barWidth,
      startUs: bar.start_us,
      incomplete: data.isIncomplete(bar),
      cells,
      stacks: bar.stacks.map((s) => ({
        side: s.side,
        y0: rowTop(rowIndex(s.high, vp.step)),
        y1: rowTop(rowIndex(s.low, vp.step)) + vp.rowHeight,
      })),
      delta: bar.delta,
      total: bar.total,
      trades: bar.trades,
    });
  }
  if (maxVolume > 0) {
    for (const cell of columns.flatMap((c) => c.cells)) {
      cell.buyShare = Number(cell.buy) / maxVolume;
      cell.sellShare = Number(cell.sell) / maxVolume;
    }
  }

  const gapBands: GapBand[] = [];
  for (const gap of data.gaps()) {
    const x0 = Math.max(0, x(gap.start_us));
    const x1 = Math.min(plotWidth, gap.end_us === null ? plotWidth : x(gap.end_us));
    if (x1 > x0) gapBands.push({ x0, x1, label: `No data: ${gap.reason.replaceAll("_", " ")}` });
  }

  return { columns, gapBands, plotWidth, plotHeight };
}
