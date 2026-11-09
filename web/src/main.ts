// Wires the page: controls ⇄ URL, history pages + live updates → ChartData,
// pointer/wheel interaction → Viewport, and a single animation-frame painter.

import { type BarQuery, type Market, fetchFootprint, fetchMarkets, liveUrl } from "./api";
import { ChartData } from "./chartData";
import { dateTimeUtc, decimals } from "./format";
import { paint, prepare } from "./render";
import { buildScene, rowIndex, type Viewport } from "./scene";
import { GROUPINGS, TIMEFRAMES, type View, parseView, serializeView } from "./urlState";

const HISTORY_BARS = 120;
const LIVE_RETRY_MS = 1_000;

function element<T extends HTMLElement>(id: string): T {
  const found = document.getElementById(id);
  if (!found) throw new Error(`#${id} is missing`);
  return found as T;
}

const canvas = element<HTMLCanvasElement>("chart");
const marketSelect = element<HTMLSelectElement>("market");
const timeframeSelect = element<HTMLSelectElement>("timeframe");
const groupingSelect = element<HTMLSelectElement>("grouping");
const ratioSelect = element<HTMLSelectElement>("ratio");
const status = element<HTMLSpanElement>("status");
const notice = element<HTMLParagraphElement>("notice");

let view: View = parseView(location.search);
let markets: Market[] = [];
const data = new ChartData();
/** Bumped on every reload; stale async work checks it and stops. */
let generation = 0;
let socket: WebSocket | null = null;
let loadingOlder = false;
let reachedHorizon = false;
/** Keep the newest bar pinned to the right edge while live updates arrive. */
let following = true;

const vp: Viewport = {
  width: 0,
  height: 0,
  axisWidth: 76,
  footerHeight: 44,
  barWidth: 128,
  rowHeight: 16,
  timeframeUs: view.timeframeSecs * 1e6,
  rightEdgeUs: 0,
  topRow: 0,
  step: 1,
};
let priceDecimals = 1;

function setStatus(text: string, state: "live" | "reconnecting" | "idle" = "idle"): void {
  status.textContent = text;
  status.dataset.state = state;
}

function showNotice(text: string | null): void {
  notice.hidden = text === null;
  notice.textContent = text ?? "";
}

function query(): BarQuery {
  return {
    market: view.market ?? "",
    timeframeSecs: view.timeframeSecs,
    grouping: view.grouping,
    imbalanceRatio: view.imbalanceRatio,
  };
}

function currentMarket(): Market | undefined {
  return markets.find((m) => m.id === view.market);
}

// ---------------------------------------------------------------- drawing

let frameRequested = false;

function requestDraw(): void {
  if (frameRequested) return;
  frameRequested = true;
  requestAnimationFrame(() => {
    frameRequested = false;
    draw();
  });
}

function draw(): void {
  const rect = canvas.getBoundingClientRect();
  vp.width = rect.width;
  vp.height = rect.height;
  if (vp.width === 0 || vp.height === 0) return;
  const ctx = prepare(canvas, vp.width, vp.height);
  const scene = buildScene(data, vp);
  paint(ctx, scene, vp, {
    priceLabel: (row) => (row * vp.step).toFixed(priceDecimals),
    horizonUs: currentMarket()?.recording_horizon_us ?? null,
  });
  maybeLoadOlder();
}

new ResizeObserver(requestDraw).observe(canvas);

// ---------------------------------------------------------------- data

async function reload(): Promise<void> {
  const myGeneration = ++generation;
  socket?.close();
  socket = null;
  data.clear();
  reachedHorizon = false;
  following = true;
  vp.timeframeUs = view.timeframeSecs * 1e6;
  history.replaceState(null, "", serializeView(view));
  syncControls();
  const market = currentMarket();
  if (!market) {
    showNotice("No Market is being served.");
    return;
  }

  const nowUs = Date.now() * 1000;
  const currentBarStart = Math.floor(nowUs / vp.timeframeUs) * vp.timeframeUs;
  vp.rightEdgeUs = currentBarStart + vp.timeframeUs;
  setStatus("Loading…");
  try {
    const page = await fetchFootprint(
      query(),
      currentBarStart - HISTORY_BARS * vp.timeframeUs,
      currentBarStart + vp.timeframeUs,
    );
    if (myGeneration !== generation) return;
    vp.step = Number(page.grouping.step);
    priceDecimals = decimals(market.tick_size);
    data.addHistory(page.bars, page.gaps);
    centerOnLatestPrice();
    showNotice(
      page.bars.length === 0
        ? market.recording_horizon_us === null
          ? "Nothing recorded for this Market yet. Waiting for live Trades…"
          : `No Trades in this range. Recording began ${dateTimeUtc(market.recording_horizon_us)}.`
        : null,
    );
  } catch (error) {
    if (myGeneration !== generation) return;
    showNotice(`Could not load history: ${(error as Error).message}`);
  }
  connectLive(myGeneration, currentBarStart);
  requestDraw();
}

/** Centres the price axis on the newest bar's middle row. */
function centerOnLatestPrice(): void {
  const latest = data.bars().at(-1);
  const rows = latest?.rows ?? [];
  const low = rows[0];
  const high = rows.at(-1);
  if (!low || !high) return;
  const middle = (rowIndex(low.price, vp.step) + rowIndex(high.price, vp.step)) / 2;
  const plotRows = (vp.height - vp.footerHeight) / vp.rowHeight || 40;
  vp.topRow = Math.round(middle + plotRows / 2);
}

function connectLive(myGeneration: number, sinceUs: number): void {
  if (myGeneration !== generation) return;
  const ws = new WebSocket(liveUrl(query(), sinceUs));
  socket = ws;
  ws.onopen = () => setStatus("Live", "live");
  ws.onmessage = (event: MessageEvent<string>) => {
    const message = JSON.parse(event.data) as Parameters<ChartData["applyLive"]>[0];
    const hadBars = data.bars().length > 0;
    data.applyLive(message);
    if (message.type === "bar") {
      if (!hadBars) {
        showNotice(null);
        centerOnLatestPrice();
      }
      if (following) vp.rightEdgeUs = Math.max(vp.rightEdgeUs, message.bar.end_us);
    }
    requestDraw();
  };
  ws.onclose = () => {
    if (myGeneration !== generation) return;
    setStatus("Reconnecting…", "reconnecting");
    // Resume from the newest bar we hold; the server replays it in full.
    const resumeFrom = data.bars().at(-1)?.start_us ?? sinceUs;
    setTimeout(() => connectLive(myGeneration, resumeFrom), LIVE_RETRY_MS);
  };
}

/** Fetches the page before the oldest bar once the user scrolls near it. */
function maybeLoadOlder(): void {
  const oldest = data.oldestStart();
  const market = currentMarket();
  if (loadingOlder || reachedHorizon || oldest === null || !market) return;
  const plotWidth = vp.width - vp.axisWidth;
  const oldestX = plotWidth - ((vp.rightEdgeUs - oldest) / vp.timeframeUs) * vp.barWidth;
  if (oldestX < -vp.barWidth * 4) return;
  if (market.recording_horizon_us !== null && oldest <= market.recording_horizon_us) {
    reachedHorizon = true;
    return;
  }
  const myGeneration = generation;
  loadingOlder = true;
  fetchFootprint(query(), oldest - HISTORY_BARS * vp.timeframeUs, oldest)
    .then((page) => {
      if (myGeneration !== generation) return;
      if (page.bars.length === 0) reachedHorizon = true;
      data.addHistory(page.bars, page.gaps);
      requestDraw();
    })
    .catch((error: unknown) => console.warn("older history failed", error))
    .finally(() => {
      loadingOlder = false;
    });
}

// ---------------------------------------------------------------- controls

function option(select: HTMLSelectElement, value: string, label: string): void {
  const o = document.createElement("option");
  o.value = value;
  o.textContent = label;
  select.append(o);
}

function syncControls(): void {
  marketSelect.value = view.market ?? "";
  timeframeSelect.value = String(view.timeframeSecs);
  ratioSelect.value = view.imbalanceRatio;
  const market = currentMarket();
  groupingSelect.replaceChildren();
  for (const multiple of GROUPINGS) {
    const step = market ? (Number(market.tick_size) * multiple).toFixed(decimals(market.tick_size)) : "";
    option(groupingSelect, String(multiple), `${step} (×${multiple})`);
  }
  groupingSelect.value = String(view.grouping);
  if (market) {
    const horizon = market.recording_horizon_us;
    marketSelect.title = [
      horizon === null ? "Not recorded yet" : `Recording since ${dateTimeUtc(horizon)}`,
      market.burst_grouped ? "Bubbles are burst-grouped (Venue does not attribute Trades to orders)" : "",
    ]
      .filter(Boolean)
      .join("\n");
  }
}

function update(change: Partial<View>): void {
  view = { ...view, ...change };
  void reload();
}

marketSelect.onchange = () => update({ market: marketSelect.value });
timeframeSelect.onchange = () => update({ timeframeSecs: Number(timeframeSelect.value) });
groupingSelect.onchange = () => update({ grouping: Number(groupingSelect.value) });
ratioSelect.onchange = () => update({ imbalanceRatio: ratioSelect.value });

// ---------------------------------------------------------------- interaction

let drag: { x: number; y: number; rightEdgeUs: number; topRow: number } | null = null;

canvas.addEventListener("pointerdown", (event) => {
  canvas.setPointerCapture(event.pointerId);
  drag = { x: event.clientX, y: event.clientY, rightEdgeUs: vp.rightEdgeUs, topRow: vp.topRow };
});
canvas.addEventListener("pointermove", (event) => {
  if (!drag) return;
  vp.rightEdgeUs = drag.rightEdgeUs - ((event.clientX - drag.x) / vp.barWidth) * vp.timeframeUs;
  vp.topRow = drag.topRow + (event.clientY - drag.y) / vp.rowHeight;
  following = vp.rightEdgeUs >= (data.newestEnd() ?? 0);
  requestDraw();
});
canvas.addEventListener("pointerup", () => {
  drag = null;
});
canvas.addEventListener("dblclick", () => {
  following = true;
  vp.rightEdgeUs = data.newestEnd() ?? vp.rightEdgeUs;
  centerOnLatestPrice();
  requestDraw();
});
canvas.addEventListener(
  "wheel",
  (event) => {
    event.preventDefault();
    const delta = event.deltaY || event.deltaX;
    if (event.ctrlKey) {
      vp.rowHeight = clamp(vp.rowHeight * (delta > 0 ? 0.9 : 1.1), 4, 48);
    } else if (event.shiftKey) {
      vp.barWidth = clamp(vp.barWidth * (delta > 0 ? 0.9 : 1.1), 24, 320);
    } else {
      vp.rightEdgeUs += (delta / vp.barWidth) * vp.timeframeUs;
      following = vp.rightEdgeUs >= (data.newestEnd() ?? 0);
    }
    requestDraw();
  },
  { passive: false },
);

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

// ---------------------------------------------------------------- start

async function start(): Promise<void> {
  for (const t of TIMEFRAMES) option(timeframeSelect, String(t.secs), t.label);
  try {
    markets = await fetchMarkets();
  } catch (error) {
    showNotice(`Gateway unavailable: ${(error as Error).message}`);
    setStatus("Offline");
    return;
  }
  for (const m of markets) option(marketSelect, m.id, `${m.native_name} · ${m.venue} ${m.market_type}`);
  if (!markets.some((m) => m.id === view.market)) view = { ...view, market: markets[0]?.id ?? null };
  await reload();
}

void start();
