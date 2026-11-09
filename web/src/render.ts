// Paints a Scene on a canvas. All layout decisions live in scene.ts.

import { clockUtc, signedVolume, volume } from "./format";
import type { Scene, Viewport } from "./scene";

/** Colors resolved from CSS custom properties so light/dark themes apply. */
interface Palette {
  background: string;
  grid: string;
  text: string;
  muted: string;
  buy: string;
  sell: string;
  buyRgb: string;
  sellRgb: string;
  gap: string;
  horizon: string;
  /** Font families (canvas fonts cannot use CSS variables). */
  mono: string;
  sans: string;
}

function palette(element: Element): Palette {
  const css = getComputedStyle(element);
  const v = (name: string) => css.getPropertyValue(name).trim();
  return {
    background: v("--chart-bg"),
    grid: v("--chart-grid"),
    text: v("--chart-text"),
    muted: v("--chart-muted"),
    buy: v("--buy"),
    sell: v("--sell"),
    buyRgb: v("--buy-rgb"),
    sellRgb: v("--sell-rgb"),
    gap: v("--gap"),
    horizon: v("--horizon"),
    mono: v("--mono") || "monospace",
    sans: v("--sans") || "sans-serif",
  };
}

export interface Overlay {
  /** Price-axis labels: row index → text. */
  priceLabel: (row: number) => string;
  /** Recording Horizon, if inside the data. */
  horizonUs: number | null;
}

/** Sizes the canvas backing store for the device pixel ratio; returns the 2D context. */
export function prepare(canvas: HTMLCanvasElement, width: number, height: number): CanvasRenderingContext2D {
  const dpr = window.devicePixelRatio || 1;
  if (canvas.width !== Math.round(width * dpr) || canvas.height !== Math.round(height * dpr)) {
    canvas.width = Math.round(width * dpr);
    canvas.height = Math.round(height * dpr);
  }
  const ctx = canvas.getContext("2d");
  if (!ctx) throw new Error("2D canvas is not available");
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  return ctx;
}

export function paint(ctx: CanvasRenderingContext2D, scene: Scene, vp: Viewport, overlay: Overlay): void {
  const p = palette(ctx.canvas);
  const { plotWidth, plotHeight } = scene;
  ctx.fillStyle = p.background;
  ctx.fillRect(0, 0, vp.width, vp.height);
  ctx.textBaseline = "middle";

  paintPriceAxis(ctx, scene, vp, overlay, p);

  ctx.save();
  ctx.beginPath();
  ctx.rect(0, 0, plotWidth, plotHeight);
  ctx.clip();
  paintGaps(ctx, scene, p);
  for (const column of scene.columns) paintColumn(ctx, column, vp, p);
  paintHorizon(ctx, vp, overlay, plotHeight, p);
  ctx.restore();

  paintFooter(ctx, scene, vp, p);
}

function paintPriceAxis(
  ctx: CanvasRenderingContext2D,
  scene: Scene,
  vp: Viewport,
  overlay: Overlay,
  p: Palette,
): void {
  const every = Math.max(1, Math.ceil(18 / vp.rowHeight));
  const firstRow = Math.floor(vp.topRow - scene.plotHeight / vp.rowHeight) - 1;
  ctx.font = `11px ${p.mono}`;
  ctx.textAlign = "left";
  for (let row = Math.ceil(vp.topRow); row >= firstRow; row--) {
    if (row % every !== 0) continue;
    const y = (vp.topRow - row - 0.5) * vp.rowHeight;
    if (y < 0 || y > scene.plotHeight) continue;
    ctx.strokeStyle = p.grid;
    ctx.beginPath();
    ctx.moveTo(0, Math.round(y) + 0.5);
    ctx.lineTo(scene.plotWidth, Math.round(y) + 0.5);
    ctx.stroke();
    ctx.fillStyle = p.muted;
    ctx.fillText(overlay.priceLabel(row), scene.plotWidth + 6, y);
  }
}

function paintGaps(ctx: CanvasRenderingContext2D, scene: Scene, p: Palette): void {
  for (const band of scene.gapBands) {
    ctx.fillStyle = p.gap;
    ctx.fillRect(band.x0, 0, band.x1 - band.x0, scene.plotHeight);
    // Diagonal hatching reads as "unknown", unlike an empty (quiet) area.
    ctx.save();
    ctx.beginPath();
    ctx.rect(band.x0, 0, band.x1 - band.x0, scene.plotHeight);
    ctx.clip();
    ctx.strokeStyle = p.grid;
    ctx.lineWidth = 1;
    for (let x = band.x0 - scene.plotHeight; x < band.x1; x += 10) {
      ctx.beginPath();
      ctx.moveTo(x, scene.plotHeight);
      ctx.lineTo(x + scene.plotHeight, 0);
      ctx.stroke();
    }
    ctx.restore();
    if (band.x1 - band.x0 > 60) {
      ctx.fillStyle = p.text;
      ctx.font = `12px ${p.sans}`;
      ctx.textAlign = "center";
      ctx.fillText(band.label, (band.x0 + band.x1) / 2, 14);
    }
  }
}

function paintColumn(
  ctx: CanvasRenderingContext2D,
  column: Scene["columns"][number],
  vp: Viewport,
  p: Palette,
): void {
  const pad = 2;
  const half = (column.width - pad * 2) / 2;
  const mid = column.x + pad + half;
  const showText = vp.rowHeight >= 11 && column.width >= 64;
  ctx.font = `${Math.min(12, vp.rowHeight - 3)}px ${p.mono}`;

  for (const cell of column.cells) {
    const y = cell.y + 0.5;
    const h = cell.height - 1;
    // Sell on the left half, buy on the right, shaded by volume.
    ctx.fillStyle = `rgba(${p.sellRgb}, ${0.08 + 0.6 * cell.sellShare})`;
    ctx.fillRect(column.x + pad, y, half, h);
    ctx.fillStyle = `rgba(${p.buyRgb}, ${0.08 + 0.6 * cell.buyShare})`;
    ctx.fillRect(mid, y, half, h);

    const sellImbalance = cell.imbalance === "sell" || cell.imbalance === "both";
    const buyImbalance = cell.imbalance === "buy" || cell.imbalance === "both";
    ctx.lineWidth = 1.5;
    if (sellImbalance) {
      ctx.strokeStyle = p.sell;
      ctx.strokeRect(column.x + pad + 0.75, y + 0.75, half - 1.5, h - 1.5);
    }
    if (buyImbalance) {
      ctx.strokeStyle = p.buy;
      ctx.strokeRect(mid + 0.75, y + 0.75, half - 1.5, h - 1.5);
    }
    if (showText) {
      const ty = cell.y + cell.height / 2;
      ctx.textAlign = "right";
      ctx.fillStyle = sellImbalance ? p.sell : p.text;
      ctx.fillText(volume(cell.sell), mid - 4, ty);
      ctx.textAlign = "left";
      ctx.fillStyle = buyImbalance ? p.buy : p.text;
      ctx.fillText(volume(cell.buy), mid + 4, ty);
    }
  }

  // Stacked Imbalances: a bar along the dominant side's edge.
  for (const stack of column.stacks) {
    ctx.fillStyle = stack.side === "buy" ? p.buy : p.sell;
    const x = stack.side === "buy" ? column.x + column.width - pad - 3 : column.x + pad;
    ctx.fillRect(x, stack.y0, 3, stack.y1 - stack.y0);
  }

  if (column.incomplete) {
    ctx.save();
    ctx.setLineDash([4, 3]);
    ctx.strokeStyle = p.muted;
    ctx.lineWidth = 1;
    ctx.strokeRect(column.x + 0.5, 0.5, column.width - 1, vp.height - vp.footerHeight - 1);
    ctx.restore();
  }
}

function paintHorizon(
  ctx: CanvasRenderingContext2D,
  vp: Viewport,
  overlay: Overlay,
  plotHeight: number,
  p: Palette,
): void {
  if (overlay.horizonUs === null) return;
  const plotWidth = vp.width - vp.axisWidth;
  const x = plotWidth - ((vp.rightEdgeUs - overlay.horizonUs) / vp.timeframeUs) * vp.barWidth;
  if (x < 0 || x > plotWidth) return;
  ctx.strokeStyle = p.horizon;
  ctx.setLineDash([6, 4]);
  ctx.beginPath();
  ctx.moveTo(Math.round(x) + 0.5, 0);
  ctx.lineTo(Math.round(x) + 0.5, plotHeight);
  ctx.stroke();
  ctx.setLineDash([]);
  ctx.fillStyle = p.horizon;
  ctx.textAlign = "left";
  ctx.font = `11px ${p.sans}`;
  ctx.fillText("Recording starts", x + 4, plotHeight - 10);
}

function paintFooter(ctx: CanvasRenderingContext2D, scene: Scene, vp: Viewport, p: Palette): void {
  const top = scene.plotHeight;
  ctx.fillStyle = p.background;
  ctx.fillRect(0, top, vp.width, vp.footerHeight);
  ctx.strokeStyle = p.grid;
  ctx.beginPath();
  ctx.moveTo(0, top + 0.5);
  ctx.lineTo(vp.width, top + 0.5);
  ctx.stroke();
  ctx.font = `11px ${p.mono}`;
  ctx.textAlign = "center";
  for (const column of scene.columns) {
    const cx = column.x + column.width / 2;
    if (cx < 0 || cx > scene.plotWidth) continue;
    const delta = Number(column.delta);
    ctx.fillStyle = delta > 0 ? p.buy : delta < 0 ? p.sell : p.text;
    ctx.fillText(`Δ ${signedVolume(column.delta)}`, cx, top + 11);
    ctx.fillStyle = p.muted;
    const label = `${volume(column.total)} · ${clockUtc(column.startUs)}${column.incomplete ? " · incomplete" : ""}`;
    ctx.fillText(label, cx, top + 28);
  }
}
