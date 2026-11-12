import { signedVolume, usd, volume } from "./format";
import {
  type BubbleShape,
  type CandleShape,
  type ColumnShape,
  type HeatmapShape,
  type LadderShape,
  type LastPriceShape,
  type Scene,
  cvdValueAt,
} from "./scene";
import type { Camera } from "./viewport";
import { priceOfY, timeOfX, xOfTime } from "./viewport";

export interface Palette {
  background: string;
  panel: string;
  grid: string;
  axis: string;
  text: string;
  muted: string;
  buy: string;
  sell: string;
  buyRgb: string;
  sellRgb: string;
  accent: string;
  accentRgb: string;
  gap: string;
  horizon: string;
  crosshair: string;
  labelBg: string;
  labelText: string;
  heatLow: string;
  heatHigh: string;
  mono: string;
  sans: string;
}

export function palette(element: Element): Palette {
  const css = getComputedStyle(element);
  const v = (name: string) => css.getPropertyValue(name).trim();
  return {
    background: v("--chart-bg"),
    panel: v("--chart-panel"),
    grid: v("--chart-grid"),
    axis: v("--chart-axis"),
    text: v("--chart-text"),
    muted: v("--chart-muted"),
    buy: v("--buy"),
    sell: v("--sell"),
    buyRgb: v("--buy-rgb"),
    sellRgb: v("--sell-rgb"),
    accent: v("--accent"),
    accentRgb: v("--accent-rgb"),
    gap: v("--gap"),
    horizon: v("--horizon"),
    crosshair: v("--crosshair"),
    labelBg: v("--label-bg"),
    labelText: v("--label-text"),
    heatLow: v("--heat-low") || "36, 68, 150",
    heatHigh: v("--heat-high") || "255, 214, 92",
    mono: v("--mono") || "monospace",
    sans: v("--sans") || "sans-serif",
  };
}

export interface Overlay {
  horizonUs: number | null;
  crosshair: { x: number; y: number } | null;
  priceDecimals: number;
}

export function prepare(canvas: HTMLCanvasElement, width: number, height: number): CanvasRenderingContext2D {
  const dpr = window.devicePixelRatio || 1;
  const w = Math.round(width * dpr);
  const h = Math.round(height * dpr);
  if (canvas.width !== w || canvas.height !== h) {
    canvas.width = w;
    canvas.height = h;
  }
  const ctx = canvas.getContext("2d");
  if (!ctx) throw new Error("2D canvas is not available");
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  return ctx;
}

const crisp = (v: number) => Math.round(v) + 0.5;

export function paint(
  ctx: CanvasRenderingContext2D,
  scene: Scene,
  cam: Camera,
  width: number,
  height: number,
  overlay: Overlay,
  p: Palette,
): void {
  const { panes } = scene;
  ctx.fillStyle = p.background;
  ctx.fillRect(0, 0, width, height);
  ctx.textBaseline = "middle";

  clipped(ctx, 0, panes.main.top, panes.plotWidth, panes.main.height, () => {
    if (scene.heatmap) paintHeatmap(ctx, scene.heatmap, p);
    paintPriceGrid(ctx, scene, p);
    paintTimeGrid(ctx, scene, panes.main.top, panes.main.height, p);
    paintGaps(ctx, scene, panes.main.top, panes.main.height, true, p);
    paintDayResets(ctx, scene, panes.main.top, panes.main.height, p);
    if (scene.columns.some((c) => c.cells.length > 0)) {
      for (const column of scene.columns) paintColumn(ctx, column, scene, cam, p);
    }
    for (const candle of scene.candles) paintCandle(ctx, candle, p);
    paintLine(ctx, scene.line, p);
    paintHorizon(ctx, cam, panes.plotWidth, panes.main.height, overlay.horizonUs, p);
    const faint = scene.mode === "numbers" && scene.columns.some((c) => c.cells.length > 0);
    for (const bubble of scene.bubbles) paintBubble(ctx, bubble, faint, p);
    if (scene.lastPrice) paintLastPriceLine(ctx, scene.lastPrice, panes.plotWidth, p);
  });
  paintStats(ctx, scene, p);
  if (scene.cvd) paintCvd(ctx, scene, p);
  paintPriceAxis(ctx, scene, width, p);
  if (scene.lastPrice) paintLastPriceTag(ctx, scene, scene.lastPrice, p);
  if (scene.ladder) paintLadder(ctx, scene, scene.ladder, cam, p);
  paintTimeAxis(ctx, scene, width, p);
  if (overlay.crosshair) paintCrosshair(ctx, scene, cam, width, overlay, p);
}

function hatch(ctx: CanvasRenderingContext2D, x: number, y: number, w: number, h: number, p: Palette): void {
  if (w <= 0 || h <= 0) return;
  ctx.fillStyle = p.gap;
  ctx.fillRect(x, y, w, h);
  clipped(ctx, x, y, w, h, () => {
    ctx.strokeStyle = p.grid;
    ctx.lineWidth = 1;
    ctx.beginPath();
    for (let sx = x - h; sx < x + w; sx += 9) {
      ctx.moveTo(sx, y + h);
      ctx.lineTo(sx + h, y);
    }
    ctx.stroke();
  });
}

const HEAT_LEVELS = 64;

function heatRamp(p: Palette): string[] {
  const low = p.heatLow.split(",").map(Number);
  const high = p.heatHigh.split(",").map(Number);
  const ramp: string[] = [];
  for (let level = 0; level <= HEAT_LEVELS; level++) {
    const v = level / HEAT_LEVELS;
    const mix = (i: number) => Math.round((low[i] ?? 0) + ((high[i] ?? 0) - (low[i] ?? 0)) * v);
    ramp.push(`rgba(${mix(0)}, ${mix(1)}, ${mix(2)}, ${(0.18 + 0.72 * v).toFixed(3)})`);
  }
  return ramp;
}

function paintHeatmap(ctx: CanvasRenderingContext2D, heatmap: HeatmapShape, p: Palette): void {
  for (const band of heatmap.unknown) hatch(ctx, band.x0, band.y0, band.x1 - band.x0, band.y1 - band.y0, p);
  const ramp = heatRamp(p);
  for (const cell of heatmap.cells) {
    ctx.fillStyle = ramp[Math.round(Math.min(1, Math.max(0, cell.v)) * HEAT_LEVELS)] ?? ramp[0]!;
    ctx.fillRect(cell.x, cell.y, cell.w + 0.5, cell.h + 0.5);
  }
}

function paintCandle(ctx: CanvasRenderingContext2D, candle: CandleShape, p: Palette): void {
  const color = candle.up ? p.buy : p.sell;
  const cx = candle.x + candle.width / 2;
  ctx.strokeStyle = color;
  ctx.lineWidth = 1;
  ctx.beginPath();
  ctx.moveTo(crisp(cx), candle.highY);
  ctx.lineTo(crisp(cx), candle.lowY);
  ctx.stroke();
  const pad = candle.width >= 8 ? Math.max(1, candle.width * 0.22) : 0.5;
  const top = Math.min(candle.open, candle.close);
  const height = Math.max(1, Math.abs(candle.close - candle.open));
  ctx.fillStyle = color;
  ctx.fillRect(candle.x + pad, top, Math.max(1, candle.width - pad * 2), height);
}

function paintLine(ctx: CanvasRenderingContext2D, points: readonly { x: number; y: number }[], p: Palette): void {
  if (points.length === 0) return;
  ctx.save();
  ctx.strokeStyle = p.text;
  ctx.lineWidth = 1.6;
  ctx.lineJoin = "round";
  ctx.beginPath();
  points.forEach((pt, i) => (i === 0 ? ctx.moveTo(pt.x, pt.y) : ctx.lineTo(pt.x, pt.y)));
  ctx.stroke();
  ctx.restore();
}

function paintLastPriceLine(ctx: CanvasRenderingContext2D, last: LastPriceShape, plotWidth: number, p: Palette): void {
  ctx.save();
  ctx.strokeStyle = last.up ? p.buy : p.sell;
  ctx.lineWidth = 1;
  ctx.setLineDash([5, 4]);
  ctx.beginPath();
  ctx.moveTo(0, crisp(last.y));
  ctx.lineTo(plotWidth, crisp(last.y));
  ctx.stroke();
  ctx.restore();
}

function paintLastPriceTag(ctx: CanvasRenderingContext2D, scene: Scene, last: LastPriceShape, p: Palette): void {
  const { plotWidth, main } = scene.panes;
  if (last.y < 0 || last.y > main.height) return;
  const w = scene.panes.dom.x - plotWidth;
  ctx.fillStyle = last.up ? p.buy : p.sell;
  ctx.fillRect(plotWidth + 1, last.y - 9, w - 2, 18);
  ctx.font = `600 11px ${p.mono}`;
  ctx.fillStyle = p.labelText;
  ctx.textAlign = "left";
  ctx.fillText(last.label, plotWidth + 8, last.y);
}

function paintLadder(ctx: CanvasRenderingContext2D, scene: Scene, ladder: LadderShape, cam: Camera, p: Palette): void {
  const { dom, main, timeAxis } = scene.panes;
  if (dom.width <= 0) return;
  ctx.fillStyle = p.panel;
  ctx.fillRect(dom.x, 0, dom.width, timeAxis.top);
  line(ctx, dom.x, 0, dom.x, timeAxis.top, p.axis);
  clipped(ctx, dom.x, main.top, dom.width, main.height, () => {
    for (const band of ladder.unknown) hatch(ctx, dom.x, band.y0, dom.width, band.y1 - band.y0, p);
    const inner = dom.width - 8;
    const showText = cam.rowHeight >= 11;
    ctx.font = `10px ${p.mono}`;
    ctx.textAlign = "right";
    for (const row of ladder.rows) {
      const h = Math.max(1, row.height - (row.height >= 6 ? 1 : 0));
      if (row.bidShare > 0) {
        ctx.fillStyle = `rgba(${p.buyRgb}, 0.55)`;
        ctx.fillRect(dom.x + 4, row.y + 0.5, inner * row.bidShare, h);
      }
      if (row.askShare > 0) {
        ctx.fillStyle = `rgba(${p.sellRgb}, 0.55)`;
        ctx.fillRect(dom.x + 4, row.y + 0.5, inner * row.askShare, h);
      }
      if (showText) {
        ctx.fillStyle = p.text;
        const text = row.askShare > 0 && row.bidShare === 0 ? volume(row.ask) : row.bidShare > 0 && row.askShare === 0 ? volume(row.bid) : `${volume(row.bid)} / ${volume(row.ask)}`;
        ctx.fillText(text, dom.x + dom.width - 6, row.y + row.height / 2);
      }
    }
    if (ladder.status !== "live") {
      ctx.fillStyle = p.gap;
      ctx.fillRect(dom.x, 0, dom.width, main.height);
      ctx.font = `600 11px ${p.sans}`;
      ctx.textAlign = "center";
      ctx.fillStyle = p.horizon;
      ctx.fillText(ladder.status === "resyncing" ? "Resyncing…" : "Disconnected", dom.x + dom.width / 2, main.height / 2);
    }
  });
}

function clipped(ctx: CanvasRenderingContext2D, x: number, y: number, w: number, h: number, draw: () => void): void {
  if (w <= 0 || h <= 0) return;
  ctx.save();
  ctx.beginPath();
  ctx.rect(x, y, w, h);
  ctx.clip();
  draw();
  ctx.restore();
}

function paintPriceGrid(ctx: CanvasRenderingContext2D, scene: Scene, p: Palette): void {
  ctx.strokeStyle = p.grid;
  ctx.lineWidth = 1;
  ctx.beginPath();
  for (const tick of scene.priceTicks) {
    ctx.moveTo(0, crisp(tick.pos));
    ctx.lineTo(scene.panes.plotWidth, crisp(tick.pos));
  }
  ctx.stroke();
}

function paintTimeGrid(ctx: CanvasRenderingContext2D, scene: Scene, top: number, height: number, p: Palette): void {
  ctx.strokeStyle = p.grid;
  ctx.lineWidth = 1;
  ctx.beginPath();
  for (const tick of scene.timeTicks) {
    ctx.moveTo(crisp(tick.pos), top);
    ctx.lineTo(crisp(tick.pos), top + height);
  }
  ctx.stroke();
}

function paintGaps(
  ctx: CanvasRenderingContext2D,
  scene: Scene,
  top: number,
  height: number,
  label: boolean,
  p: Palette,
): void {
  for (const band of scene.gapBands) {
    const w = band.x1 - band.x0;
    ctx.fillStyle = p.gap;
    ctx.fillRect(band.x0, top, w, height);
    clipped(ctx, band.x0, top, w, height, () => {
      ctx.strokeStyle = p.grid;
      ctx.lineWidth = 1;
      ctx.beginPath();
      for (let x = band.x0 - height; x < band.x1; x += 9) {
        ctx.moveTo(x, top + height);
        ctx.lineTo(x + height, top);
      }
      ctx.stroke();
    });
    if (label && w > 90) {
      ctx.font = `600 11px ${p.sans}`;
      ctx.textAlign = "center";
      ctx.fillStyle = p.muted;
      ctx.fillText(band.label, band.x0 + w / 2, top + 14);
    }
  }
}

function paintDayResets(ctx: CanvasRenderingContext2D, scene: Scene, top: number, height: number, p: Palette): void {
  if (scene.dayResets.length === 0) return;
  ctx.save();
  ctx.strokeStyle = p.horizon;
  ctx.setLineDash([2, 4]);
  ctx.beginPath();
  for (const x of scene.dayResets) {
    ctx.moveTo(crisp(x), top);
    ctx.lineTo(crisp(x), top + height);
  }
  ctx.stroke();
  ctx.restore();
}

function paintColumn(ctx: CanvasRenderingContext2D, column: ColumnShape, scene: Scene, cam: Camera, p: Palette): void {
  const pad = column.width >= 12 ? 1.5 : 0.5;
  const inner = column.width - pad * 2;
  const mid = column.x + pad + inner / 2;
  const numbers = scene.mode === "numbers";
  const fontSize = Math.min(12, Math.max(8, cam.rowHeight - 4));
  ctx.font = `${fontSize}px ${p.mono}`;

  for (const cell of column.cells) {
    const y = cell.y + 0.5;
    const h = Math.max(1, cell.height - (cam.rowHeight >= 6 ? 1 : 0));
    const sellImbalance = cell.imbalance === "sell" || cell.imbalance === "both";
    const buyImbalance = cell.imbalance === "buy" || cell.imbalance === "both";
    if (numbers) {
      ctx.fillStyle = `rgba(${p.sellRgb}, ${0.06 + 0.55 * cell.sellShare})`;
      ctx.fillRect(column.x + pad, y, inner / 2, h);
      ctx.fillStyle = `rgba(${p.buyRgb}, ${0.06 + 0.55 * cell.buyShare})`;
      ctx.fillRect(mid, y, inner / 2, h);
    } else {
      const sellWidth = (inner / 2) * Math.max(cell.sellShare, cell.sell === "0" ? 0 : 0.04);
      const buyWidth = (inner / 2) * Math.max(cell.buyShare, cell.buy === "0" ? 0 : 0.04);
      ctx.fillStyle = sellImbalance ? p.sell : `rgba(${p.sellRgb}, 0.55)`;
      ctx.fillRect(mid - sellWidth, y, sellWidth, h);
      ctx.fillStyle = buyImbalance ? p.buy : `rgba(${p.buyRgb}, 0.55)`;
      ctx.fillRect(mid, y, buyWidth, h);
    }
    if (numbers) {
      ctx.lineWidth = 1.5;
      if (sellImbalance) {
        ctx.strokeStyle = p.sell;
        ctx.strokeRect(column.x + pad + 0.75, y + 0.75, inner / 2 - 1.5, h - 1.5);
      }
      if (buyImbalance) {
        ctx.strokeStyle = p.buy;
        ctx.strokeRect(mid + 0.75, y + 0.75, inner / 2 - 1.5, h - 1.5);
      }
      const ty = cell.y + cell.height / 2;
      ctx.textAlign = "right";
      ctx.fillStyle = sellImbalance ? p.sell : p.text;
      ctx.fillText(volume(cell.sell), mid - 4, ty);
      ctx.textAlign = "left";
      ctx.fillStyle = buyImbalance ? p.buy : p.text;
      ctx.fillText(volume(cell.buy), mid + 4, ty);
    }
  }

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
    ctx.strokeRect(crisp(column.x), 0.5, column.width - 1, scene.panes.main.height - 1);
    ctx.restore();
  }
}

function paintHorizon(
  ctx: CanvasRenderingContext2D,
  cam: Camera,
  plotWidth: number,
  height: number,
  horizonUs: number | null,
  p: Palette,
): void {
  if (horizonUs === null) return;
  const x = xOfTime(cam, plotWidth, horizonUs);
  if (x < 0 || x > plotWidth) return;
  ctx.save();
  ctx.strokeStyle = p.horizon;
  ctx.setLineDash([6, 4]);
  ctx.beginPath();
  ctx.moveTo(crisp(x), 0);
  ctx.lineTo(crisp(x), height);
  ctx.stroke();
  ctx.restore();
  ctx.fillStyle = p.horizon;
  ctx.textAlign = "left";
  ctx.font = `11px ${p.sans}`;
  ctx.fillText("Recording starts", x + 5, height - 12);
}

function paintBubble(ctx: CanvasRenderingContext2D, shape: BubbleShape, faint: boolean, p: Palette): void {
  const { bubble, x, y, radius } = shape;
  const rgb = bubble.side === "buy" ? p.buyRgb : p.sellRgb;
  const solid = bubble.side === "buy" ? p.buy : p.sell;
  const alpha = (bubble.large ? 0.62 : 0.32) * (faint ? 0.6 : 1) * shape.alpha;
  ctx.beginPath();
  ctx.arc(x, y, radius, 0, Math.PI * 2);
  ctx.fillStyle = `rgba(${rgb}, ${alpha})`;
  ctx.fill();
  ctx.lineWidth = bubble.large ? 2 : 1;
  ctx.strokeStyle = bubble.large ? p.text : solid;
  ctx.stroke();
  if (bubble.large && radius >= 10) {
    ctx.font = `600 11px ${p.sans}`;
    ctx.textAlign = "center";
    ctx.fillStyle = p.labelText;
    ctx.fillText(usd(bubble.notional), x, y);
  }
}

function paintStats(ctx: CanvasRenderingContext2D, scene: Scene, p: Palette): void {
  const { stats, plotWidth } = scene.panes;
  if (stats.height <= 0) return;
  ctx.fillStyle = p.panel;
  ctx.fillRect(0, stats.top, plotWidth, stats.height);
  line(ctx, 0, stats.top, plotWidth, stats.top, p.axis);
  ctx.font = `11px ${p.mono}`;
  ctx.textAlign = "center";
  for (const column of scene.columns) {
    const cx = column.x + column.width / 2;
    if (cx < 0 || cx > plotWidth || column.width < 30) continue;
    const delta = Number(column.bar.delta);
    ctx.fillStyle = delta > 0 ? p.buy : delta < 0 ? p.sell : p.muted;
    ctx.fillText(signedVolume(column.bar.delta), cx, stats.top + stats.height * 0.32);
    ctx.fillStyle = p.muted;
    ctx.fillText(volume(column.bar.total), cx, stats.top + stats.height * 0.74);
  }
}

function paintCvd(ctx: CanvasRenderingContext2D, scene: Scene, p: Palette): void {
  const cvd = scene.cvd;
  const { cvd: band, plotWidth } = scene.panes;
  if (!cvd) return;
  ctx.fillStyle = p.panel;
  ctx.fillRect(0, band.top, plotWidth, band.height);
  line(ctx, 0, band.top, plotWidth, band.top, p.axis);
  clipped(ctx, 0, band.top, plotWidth, band.height, () => {
    paintTimeGrid(ctx, scene, band.top, band.height, p);
    paintGaps(ctx, scene, band.top, band.height, false, p);
    paintDayResets(ctx, scene, band.top, band.height, p);
    for (const tick of cvd.ticks) line(ctx, 0, tick.pos, plotWidth, tick.pos, p.grid);
    if (cvd.zeroY !== null) line(ctx, 0, cvd.zeroY, plotWidth, cvd.zeroY, p.muted);
    for (const segment of cvd.segments) {
      ctx.save();
      ctx.lineWidth = 1.75;
      ctx.lineJoin = "round";
      ctx.strokeStyle = segment.complete ? p.accent : `rgba(${p.accentRgb}, 0.6)`;
      if (!segment.complete) ctx.setLineDash([6, 4]);
      ctx.beginPath();
      segment.points.forEach((pt, i) => (i === 0 ? ctx.moveTo(pt.x, pt.y) : ctx.lineTo(pt.x, pt.y)));
      ctx.stroke();
      ctx.restore();
    }
    const last = cvd.points.at(-1);
    if (last) {
      ctx.beginPath();
      ctx.arc(last.x, last.y, 3, 0, Math.PI * 2);
      ctx.fillStyle = p.accent;
      ctx.fill();
    }
  });
  ctx.font = `600 10px ${p.sans}`;
  ctx.textAlign = "left";
  ctx.fillStyle = p.muted;
  const incomplete = cvd.points.at(-1)?.complete === false;
  ctx.fillText(incomplete ? "CVD · incomplete: a Gap today, values exclude unknown Trades" : "CVD", 8, band.top + 11);
}

function paintPriceAxis(ctx: CanvasRenderingContext2D, scene: Scene, width: number, p: Palette): void {
  const { plotWidth, main, cvd, stats, timeAxis } = scene.panes;
  ctx.fillStyle = p.panel;
  ctx.fillRect(plotWidth, 0, width - plotWidth, timeAxis.top);
  ctx.font = `11px ${p.mono}`;
  line(ctx, plotWidth, 0, plotWidth, timeAxis.top, p.axis);
  ctx.font = `11px ${p.mono}`;
  ctx.textAlign = "left";
  ctx.fillStyle = p.muted;
  for (const tick of scene.priceTicks) {
    if (tick.pos < 6 || tick.pos > main.height - 6) continue;
    ctx.fillText(tick.label, plotWidth + 8, tick.pos);
  }
  if (stats.height > 0) line(ctx, plotWidth, stats.top, width, stats.top, p.axis);
  if (scene.cvd) {
    line(ctx, plotWidth, cvd.top, width, cvd.top, p.axis);
    for (const tick of scene.cvd.ticks) {
      if (tick.pos < cvd.top + 6 || tick.pos > cvd.top + cvd.height - 6) continue;
      ctx.fillText(tick.label, plotWidth + 8, tick.pos);
    }
  }
}

function paintTimeAxis(ctx: CanvasRenderingContext2D, scene: Scene, width: number, p: Palette): void {
  const { timeAxis, plotWidth } = scene.panes;
  ctx.fillStyle = p.panel;
  ctx.fillRect(0, timeAxis.top, width, timeAxis.height);
  line(ctx, 0, timeAxis.top, width, timeAxis.top, p.axis);
  ctx.textAlign = "center";
  for (const tick of scene.timeTicks) {
    if (tick.pos < 20 || tick.pos > plotWidth - 20) continue;
    ctx.font = tick.major ? `600 11px ${p.sans}` : `11px ${p.mono}`;
    ctx.fillStyle = tick.major ? p.text : p.muted;
    ctx.fillText(tick.label, tick.pos, timeAxis.top + timeAxis.height / 2);
  }
}

function paintCrosshair(
  ctx: CanvasRenderingContext2D,
  scene: Scene,
  cam: Camera,
  width: number,
  overlay: Overlay,
  p: Palette,
): void {
  const { x, y } = overlay.crosshair ?? { x: -1, y: -1 };
  const { plotWidth, main, cvd, timeAxis, dom } = scene.panes;
  if (x < 0 || x > plotWidth || y < 0 || y > timeAxis.top) return;
  ctx.save();
  ctx.strokeStyle = p.crosshair;
  ctx.setLineDash([4, 4]);
  ctx.beginPath();
  ctx.moveTo(crisp(x), 0);
  ctx.lineTo(crisp(x), timeAxis.top);
  if (y < main.height || (scene.cvd && y >= cvd.top)) {
    ctx.moveTo(0, crisp(y));
    ctx.lineTo(plotWidth, crisp(y));
  }
  ctx.stroke();
  ctx.restore();

  const axisWidth = dom.x - plotWidth;
  if (y < main.height) {
    axisLabel(ctx, priceOfY(cam, y).toFixed(overlay.priceDecimals), plotWidth, y, axisWidth, p);
  } else if (scene.cvd && y >= cvd.top) {
    axisLabel(ctx, cvdValueAt(scene.cvd.scale, y).toFixed(2), plotWidth, y, axisWidth, p);
  }
  const t = timeOfX(cam, plotWidth, x);
  const label = new Date(t / 1000).toISOString().slice(5, 16).replace("T", " ");
  ctx.font = `11px ${p.mono}`;
  const w = ctx.measureText(label).width + 12;
  const left = Math.min(Math.max(0, x - w / 2), plotWidth - w);
  ctx.fillStyle = p.labelBg;
  ctx.fillRect(left, timeAxis.top + 1, w, timeAxis.height - 2);
  ctx.fillStyle = p.labelText;
  ctx.textAlign = "center";
  ctx.fillText(label, left + w / 2, timeAxis.top + timeAxis.height / 2);
}

function axisLabel(ctx: CanvasRenderingContext2D, text: string, x: number, y: number, w: number, p: Palette): void {
  ctx.fillStyle = p.labelBg;
  ctx.fillRect(x + 1, y - 9, w - 2, 18);
  ctx.font = `11px ${p.mono}`;
  ctx.fillStyle = p.labelText;
  ctx.textAlign = "left";
  ctx.fillText(text, x + 8, y);
}

function line(ctx: CanvasRenderingContext2D, x0: number, y0: number, x1: number, y1: number, color: string): void {
  ctx.strokeStyle = color;
  ctx.lineWidth = 1;
  ctx.beginPath();
  ctx.moveTo(x0, crisp(y0));
  ctx.lineTo(x1, crisp(y1));
  ctx.stroke();
}
