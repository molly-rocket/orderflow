import {
  type BookMessage,
  type ChartQuery,
  type LiveMessage,
  type Market,
  bookUrl,
  fetchBubbles,
  fetchFootprint,
  fetchMarkets,
  liveUrl,
} from "./api";
import { BookData } from "./bookData";
import { ChartData } from "./chartData";
import { clockUtc, dateTimeUtc, decimals, preciseClockUtc, price, signedVolume, usd, volume } from "./format";
import { type Hit, hitTest } from "./hit";
import { type Palette, paint, palette, prepare } from "./render";
import { type Scene, buildScene } from "./scene";
import {
  AUTO_GROUPING,
  GROUPINGS,
  IMBALANCE_RATIOS,
  PERCENTILES,
  PRICE_STYLES,
  TIMEFRAMES,
  type PriceStyle,
  type Theme,
  type ViewState,
  type Views,
  autoGrouping,
  loadPreferences,
  parseView,
  savePreferences,
  serializeView,
} from "./settings";
import {
  type Camera,
  type Layout,
  ROW_HEIGHT,
  pan,
  panes,
  priceOfY,
  timeOfX,
  zoomPrice,
  zoomTime,
} from "./viewport";

const HISTORY_BARS = 160;
const MAX_BUBBLE_SPAN_US = 3 * 86_400_000_000 - 60_000_000;
const RETRY_DELAYS_MS = [500, 1_000, 2_000, 5_000];
const WHEEL_ZOOM = 1.12;
const AUTO_SAMPLE_BARS = 30;
const AUTO_SAMPLE_GROUPING = 100;
const FIT_FILL = 0.88;
const FIT_MAX_ROW_HEIGHT = 26;
const DOM_WIDTH = 150;
const HEATMAP_LOOKBACK_US = 2 * 3_600_000_000;

function element<T extends HTMLElement>(id: string): T {
  const found = document.getElementById(id);
  if (!found) throw new Error(`#${id} is missing`);
  return found as T;
}

function safeStorage(): Storage | null {
  try {
    return window.localStorage;
  } catch {
    return null;
  }
}

const canvas = element<HTMLCanvasElement>("chart");
const tooltip = element<HTMLDivElement>("tooltip");
const notice = element<HTMLParagraphElement>("notice");
const followButton = element<HTMLButtonElement>("follow");
const statusPill = element<HTMLSpanElement>("status");
const lastPricePill = element<HTMLSpanElement>("last-price");
const bookHeader = element<HTMLDivElement>("book-header");
const priceStyleGroup = element<HTMLDivElement>("price-style");
const marketSelect = element<HTMLSelectElement>("market");
const marketMeta = element<HTMLSpanElement>("market-meta");
const timeframes = element<HTMLDivElement>("timeframes");
const groupingSelect = element<HTMLSelectElement>("grouping");
const ratioSelect = element<HTMLSelectElement>("ratio");
const stackInput = element<HTMLInputElement>("stack");
const percentileSelect = element<HTMLSelectElement>("percentile");
const floorInput = element<HTMLInputElement>("floor");
const minNotionalInput = element<HTMLInputElement>("min-notional");
const sideGroup = element<HTMLDivElement>("side");
const settingsPanel = element<HTMLElement>("settings");
const settingsToggle = element<HTMLButtonElement>("settings-toggle");
const themeToggle = element<HTMLButtonElement>("theme-toggle");
const dataNote = element<HTMLSpanElement>("data-note");

const storage = safeStorage();
const preferences = loadPreferences(storage);
let theme: Theme = preferences.theme;
let view: ViewState = parseView(location.search, preferences.view);
let markets: Market[] = [];
const data = new ChartData();
const bookData = new BookData();
let generation = 0;
let socket: WebSocket | null = null;
let bookSocket: WebSocket | null = null;
let bookGeneration = 0;
let loadingOlder = false;
let reachedHorizon = false;
let following = true;
let largeThreshold: number | null = null;
let truncatedBubbles = false;
let pointer: { x: number; y: number } | null = null;
let scene: Scene | null = null;
let colors: Palette | null = null;
let priceDecimals = 1;
let effectiveGrouping = 1;
let cam: Camera = {
  timeframeUs: view.timeframeSecs * 1e6,
  barWidth: 112,
  rightEdgeUs: 0,
  topRow: 0,
  rowHeight: 16,
  step: 1,
};

function query(grouping = effectiveGrouping): ChartQuery {
  return {
    market: view.market ?? "",
    timeframeSecs: view.timeframeSecs,
    grouping,
    imbalanceRatio: view.imbalanceRatio,
    minStack: view.minStack,
    percentile: view.percentile,
    usdFloor: view.usdFloor,
    minNotional: view.minNotional,
    side: view.side,
  };
}

function currentMarket(): Market | undefined {
  return markets.find((m) => m.id === view.market);
}

function layout(): Layout {
  const rect = canvas.getBoundingClientRect();
  return {
    width: rect.width,
    height: rect.height,
    axisWidth: 84,
    timeAxisHeight: 26,
    statsHeight: view.priceStyle === "footprint" ? 34 : 0,
    cvdHeight: view.views.cvd ? Math.round(Math.min(220, Math.max(96, rect.height * 0.24))) : 0,
    domWidth: view.views.dom ? Math.min(DOM_WIDTH, Math.round(rect.width * 0.2)) : 0,
  };
}

function setStatus(text: string, state: "live" | "reconnecting" | "offline" | "idle"): void {
  statusPill.textContent = text;
  statusPill.dataset.state = state;
}

function showNotice(text: string | null): void {
  notice.hidden = text === null;
  notice.textContent = text ?? "";
}

function persist(): void {
  history.replaceState(null, "", serializeView(view));
  savePreferences(storage, { theme, view: { ...view, market: view.market } });
}

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
  const lay = layout();
  if (lay.width === 0 || lay.height === 0) return;
  colors ??= palette(canvas);
  const ctx = prepare(canvas, lay.width, lay.height);
  scene = buildScene(
    data,
    cam,
    lay,
    {
      priceStyle: view.priceStyle,
      showBubbles: view.views.bubbles,
      showCvd: view.views.cvd,
      showDom: view.views.dom,
      showHeatmap: view.views.heatmap,
      showLastPrice: view.views.lastPrice,
      priceDecimals,
      largeThreshold,
    },
    bookData,
  );
  paint(ctx, scene, cam, lay.width, lay.height, {
    horizonUs: currentMarket()?.recording_horizon_us ?? null,
    crosshair: pointer,
    priceDecimals,
  }, colors);
  followButton.hidden = following || data.bars().length === 0;
  syncLastPrice();
  syncBookHeader();
  updateTooltip();
  maybeLoadOlder();
}

function syncLastPrice(): void {
  const last = scene?.lastPrice ?? null;
  lastPricePill.hidden = last === null;
  if (!last) return;
  lastPricePill.textContent = last.label;
  lastPricePill.dataset.direction = last.up ? "up" : "down";
}

function syncBookHeader(): void {
  const ladder = scene?.ladder ?? null;
  bookHeader.hidden = ladder === null;
  if (!ladder) return;
  bookHeader.dataset.status = ladder.status;
  bookHeader.textContent =
    ladder.status === "live"
      ? ladder.header || "order book"
      : ladder.status === "resyncing"
        ? "order book resyncing"
        : "order book disconnected";
}

function wantsBook(): boolean {
  return view.views.dom || view.views.heatmap;
}

function heatmapSince(): number | null {
  if (!view.views.heatmap) return null;
  return data.oldestStart() ?? Date.now() * 1000 - HEATMAP_LOOKBACK_US;
}

function closeBook(): void {
  bookGeneration++;
  bookSocket?.close();
  bookSocket = null;
  bookData.clear();
}

function connectBook(attempt = 0): void {
  if (!view.market || !wantsBook()) return;
  const myGeneration = bookGeneration;
  const ws = new WebSocket(bookUrl(query(), heatmapSince()));
  bookSocket = ws;
  let opened = false;
  ws.onopen = () => {
    opened = true;
  };
  ws.onmessage = (event: MessageEvent<string>) => {
    if (myGeneration !== bookGeneration) return;
    const message = JSON.parse(event.data) as BookMessage;
    bookData.applyBook(message);
    if (message.type === "heatmap") bookData.prune(heatmapSince() ?? message.frame.time_us);
    requestDraw();
  };
  ws.onclose = () => {
    if (myGeneration !== bookGeneration) return;
    bookData.markDisconnected();
    requestDraw();
    const next = opened ? 0 : attempt + 1;
    const delay = RETRY_DELAYS_MS[Math.min(next, RETRY_DELAYS_MS.length - 1)] ?? 5_000;
    setTimeout(() => {
      if (myGeneration === bookGeneration) connectBook(next);
    }, delay);
  };
}

function syncBook(): void {
  closeBook();
  connectBook();
  requestDraw();
}

new ResizeObserver(requestDraw).observe(canvas);

function mainHeight(): number {
  return panes(layout()).main.height;
}

function priceSpan(fromUs: number, toUs: number): [number, number] | null {
  let low = Number.POSITIVE_INFINITY;
  let high = Number.NEGATIVE_INFINITY;
  for (const bar of data.bars()) {
    if (bar.end_us <= fromUs || bar.start_us >= toUs) continue;
    const first = bar.rows[0];
    const last = bar.rows.at(-1);
    if (!first || !last) continue;
    low = Math.min(low, Number(first.price));
    high = Math.max(high, Number(last.price) + cam.step);
  }
  return Number.isFinite(low) && Number.isFinite(high) ? [low, high] : null;
}

function fitPrice(): void {
  const plotWidth = panes(layout()).plotWidth;
  const height = mainHeight() || 600;
  const span =
    priceSpan(timeOfX(cam, plotWidth, 0), timeOfX(cam, plotWidth, plotWidth)) ??
    priceSpan(Number.NEGATIVE_INFINITY, Number.POSITIVE_INFINITY);
  if (!span) return;
  const [low, high] = span;
  const rows = Math.max(1, (high - low) / cam.step);
  const rowHeight = Math.min(FIT_MAX_ROW_HEIGHT, Math.max(ROW_HEIGHT.min, (height * FIT_FILL) / rows));
  const middle = (low + high) / 2;
  cam = { ...cam, rowHeight, topRow: middle / cam.step + height / rowHeight / 2 };
}

function keepLatestInView(): void {
  const latest = data.bars().at(-1);
  const first = latest?.rows[0];
  const last = latest?.rows.at(-1);
  if (!first || !last) return;
  const height = mainHeight();
  const top = priceOfY(cam, height * 0.05);
  const bottom = priceOfY(cam, height * 0.95);
  const high = Number(last.price) + cam.step;
  const low = Number(first.price);
  if (high > top) cam = { ...cam, topRow: cam.topRow + (high - top) / cam.step };
  else if (low < bottom) cam = { ...cam, topRow: cam.topRow - (bottom - low) / cam.step };
}

async function resolveGrouping(nowBarStart: number, timeframeUs: number): Promise<number> {
  if (view.grouping !== AUTO_GROUPING) return view.grouping;
  const sample = await fetchFootprint(
    query(AUTO_SAMPLE_GROUPING),
    nowBarStart - AUTO_SAMPLE_BARS * timeframeUs,
    nowBarStart + timeframeUs,
  );
  const tick = Number(sample.grouping.tick);
  const step = Number(sample.grouping.step);
  const ranges = sample.bars.map((bar) => {
    const first = bar.rows[0];
    const last = bar.rows.at(-1);
    return first && last ? (Number(last.price) - Number(first.price) + step) / tick : 0;
  });
  return autoGrouping(ranges);
}

function bubbleRange(fromUs: number, toUs: number): [number, number] {
  return [Math.max(fromUs, toUs - MAX_BUBBLE_SPAN_US), toUs];
}

async function reload(keepCamera = false): Promise<void> {
  const myGeneration = ++generation;
  socket?.close();
  socket = null;
  data.clear();
  reachedHorizon = false;
  persist();
  syncControls();
  const market = currentMarket();
  if (!market) {
    showNotice("No Market is being served.");
    return;
  }
  const timeframeUs = view.timeframeSecs * 1e6;
  const nowUs = Date.now() * 1000;
  const currentBarStart = Math.floor(nowUs / timeframeUs) * timeframeUs;
  if (!keepCamera || cam.timeframeUs !== timeframeUs) {
    following = true;
    cam = { ...cam, timeframeUs, rightEdgeUs: currentBarStart + timeframeUs * 1.5 };
  }
  setStatus("Loading…", "idle");
  const from = currentBarStart - HISTORY_BARS * timeframeUs;
  const to = currentBarStart + timeframeUs;
  try {
    effectiveGrouping = await resolveGrouping(currentBarStart, timeframeUs);
    if (myGeneration !== generation) return;
    syncGroupingOptions();
    const [page, bubbles] = await Promise.all([
      fetchFootprint(query(), from, to),
      fetchBubbles(query(), ...bubbleRange(from, to)),
    ]);
    if (myGeneration !== generation) return;
    const stepChanged = cam.step !== Number(page.grouping.step);
    cam = { ...cam, step: Number(page.grouping.step) };
    priceDecimals = Math.max(decimals(market.tick_size), decimals(page.grouping.step));
    data.addHistory(page.bars, page.gaps);
    data.addBubbles(bubbles.bubbles, bubbles.gaps);
    largeThreshold = bubbles.large.threshold === null ? null : Number(bubbles.large.threshold);
    truncatedBubbles = bubbles.truncated;
    if (!keepCamera || stepChanged) fitPrice();
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
  updateDataNote();
  connectLive(myGeneration, currentBarStart, 0);
  syncBook();
  requestDraw();
}

function connectLive(myGeneration: number, sinceUs: number, attempt: number): void {
  if (myGeneration !== generation) return;
  const ws = new WebSocket(liveUrl(query(), sinceUs));
  socket = ws;
  let opened = false;
  ws.onopen = () => {
    opened = true;
    setStatus("Live", "live");
  };
  ws.onmessage = (event: MessageEvent<string>) => {
    if (myGeneration !== generation) return;
    const message = JSON.parse(event.data) as LiveMessage;
    const hadBars = data.bars().length > 0;
    data.applyLive(message);
    if (message.type === "bar") {
      if (!hadBars) {
        showNotice(null);
        fitPrice();
      }
      if (following) {
        cam = { ...cam, rightEdgeUs: Math.max(cam.rightEdgeUs, message.bar.end_us + cam.timeframeUs * 0.5) };
        keepLatestInView();
      }
    }
    requestDraw();
  };
  ws.onclose = () => {
    if (myGeneration !== generation) return;
    setStatus("Reconnecting…", "reconnecting");
    const resumeFrom = data.bars().at(-1)?.start_us ?? sinceUs;
    const next = opened ? 0 : attempt + 1;
    const delay = RETRY_DELAYS_MS[Math.min(next, RETRY_DELAYS_MS.length - 1)] ?? 5_000;
    setTimeout(() => connectLive(myGeneration, resumeFrom, next), delay);
  };
}

function maybeLoadOlder(): void {
  const oldest = data.oldestStart();
  const market = currentMarket();
  if (loadingOlder || reachedHorizon || oldest === null || !market || !scene) return;
  const oldestX = scene.panes.plotWidth - ((cam.rightEdgeUs - oldest) / cam.timeframeUs) * cam.barWidth;
  if (oldestX < -cam.barWidth * 6) return;
  if (market.recording_horizon_us !== null && oldest <= market.recording_horizon_us) {
    reachedHorizon = true;
    return;
  }
  const myGeneration = generation;
  const from = oldest - HISTORY_BARS * cam.timeframeUs;
  loadingOlder = true;
  Promise.all([fetchFootprint(query(), from, oldest), fetchBubbles(query(), ...bubbleRange(from, oldest))])
    .then(([page, bubbles]) => {
      if (myGeneration !== generation) return;
      if (page.bars.length === 0) reachedHorizon = true;
      data.addHistory(page.bars, page.gaps);
      data.addBubbles(bubbles.bubbles, bubbles.gaps);
      requestDraw();
    })
    .catch((error: unknown) => console.warn("older history failed", error))
    .finally(() => {
      loadingOlder = false;
    });
}

function option(select: HTMLSelectElement, value: string, label: string): void {
  const o = document.createElement("option");
  o.value = value;
  o.textContent = label;
  select.append(o);
}

function updateDataNote(): void {
  const market = currentMarket();
  const notes: string[] = [];
  if (market?.burst_grouped) notes.push("Bubbles are burst-grouped: this Venue does not attribute Trades to orders");
  if (truncatedBubbles) notes.push("showing the largest Bubbles in range");
  dataNote.textContent = notes.join(" · ");
}

function syncGroupingOptions(): void {
  const market = currentMarket();
  const stepLabel = (multiple: number) =>
    market ? (Number(market.tick_size) * multiple).toFixed(decimals(market.tick_size)) : String(multiple);
  groupingSelect.replaceChildren();
  option(groupingSelect, String(AUTO_GROUPING), `Auto (${stepLabel(effectiveGrouping)})`);
  for (const multiple of GROUPINGS) option(groupingSelect, String(multiple), stepLabel(multiple));
  groupingSelect.value = String(view.grouping);
}

function syncControls(): void {
  marketSelect.value = view.market ?? "";
  for (const button of timeframes.querySelectorAll<HTMLButtonElement>("button")) {
    button.setAttribute("aria-checked", String(Number(button.dataset.tf) === view.timeframeSecs));
  }
  const market = currentMarket();
  syncGroupingOptions();
  for (const chip of document.querySelectorAll<HTMLButtonElement>(".chip[data-view]")) {
    const name = chip.dataset.view as keyof Views;
    chip.setAttribute("aria-pressed", String(view.views[name]));
  }
  for (const button of priceStyleGroup.querySelectorAll<HTMLButtonElement>("button")) {
    button.setAttribute("aria-checked", String(button.dataset.style === view.priceStyle));
  }
  ratioSelect.value = view.imbalanceRatio;
  stackInput.value = String(view.minStack);
  percentileSelect.value = view.percentile;
  floorInput.value = view.usdFloor;
  minNotionalInput.value = view.minNotional;
  for (const button of sideGroup.querySelectorAll<HTMLButtonElement>("button")) {
    button.setAttribute("aria-checked", String(button.dataset.side === view.side));
  }
  if (market) {
    const horizon = market.recording_horizon_us;
    marketMeta.textContent = `tick ${market.tick_size}${horizon === null ? "" : ` · recording since ${dateTimeUtc(horizon)}`}`;
  }
}

function update(change: Partial<ViewState>, keepCamera = false): void {
  view = { ...view, ...change };
  void reload(keepCamera);
}

function toggleView(name: keyof Views): void {
  view = { ...view, views: { ...view.views, [name]: !view.views[name] } };
  persist();
  syncControls();
  if (name === "dom" || name === "heatmap") syncBook();
  requestDraw();
}

function setPriceStyle(priceStyle: PriceStyle): void {
  view = { ...view, priceStyle };
  persist();
  syncControls();
  requestDraw();
}

function applyTheme(next: Theme): void {
  theme = next;
  document.documentElement.dataset.theme = theme;
  colors = null;
  persist();
  requestDraw();
}

function followLive(): void {
  following = true;
  const newest = data.newestEnd();
  if (newest !== null) cam = { ...cam, rightEdgeUs: newest + cam.timeframeUs * 0.5 };
  fitPrice();
  requestDraw();
}

function buildControls(): void {
  for (const t of TIMEFRAMES) {
    const button = document.createElement("button");
    button.type = "button";
    button.role = "radio";
    button.dataset.tf = String(t.secs);
    button.textContent = t.label;
    button.onclick = () => update({ timeframeSecs: t.secs });
    timeframes.append(button);
  }
  for (const style of PRICE_STYLES) {
    const button = document.createElement("button");
    button.type = "button";
    button.role = "radio";
    button.dataset.style = style.value;
    button.dataset.price = style.value === "footprint" ? "fp" : style.value === "hidden" ? "none" : style.value;
    button.textContent = style.label;
    button.onclick = () => setPriceStyle(style.value);
    priceStyleGroup.append(button);
  }
  for (const r of IMBALANCE_RATIOS) option(ratioSelect, r.value, r.label);
  for (const p of PERCENTILES) option(percentileSelect, p.value, p.label);

  marketSelect.onchange = () => update({ market: marketSelect.value });
  groupingSelect.onchange = () => update({ grouping: Number(groupingSelect.value) });
  ratioSelect.onchange = () => update({ imbalanceRatio: ratioSelect.value }, true);
  stackInput.onchange = () => {
    const stack = Math.round(Number(stackInput.value));
    if (stack >= 2 && stack <= 20) update({ minStack: stack }, true);
    else stackInput.value = String(view.minStack);
  };
  percentileSelect.onchange = () => update({ percentile: percentileSelect.value }, true);
  floorInput.onchange = () => {
    const floor = floorInput.value.trim();
    if (floor === "" || Number(floor) >= 0) update({ usdFloor: floor }, true);
    else floorInput.value = view.usdFloor;
  };
  minNotionalInput.onchange = () => {
    const minimum = minNotionalInput.value.trim();
    if (minimum === "" || Number(minimum) >= 0) update({ minNotional: minimum === "" ? "0" : minimum }, true);
    else minNotionalInput.value = view.minNotional;
  };
  for (const button of sideGroup.querySelectorAll<HTMLButtonElement>("button")) {
    button.onclick = () => {
      const side = button.dataset.side;
      if (side === "both" || side === "buy" || side === "sell") update({ side }, true);
    };
  }
  for (const chip of document.querySelectorAll<HTMLButtonElement>(".chip[data-view]")) {
    chip.onclick = () => toggleView(chip.dataset.view as keyof Views);
  }
  themeToggle.onclick = () => applyTheme(theme === "dark" ? "light" : "dark");
  settingsToggle.onclick = () => {
    settingsPanel.hidden = !settingsPanel.hidden;
    settingsToggle.setAttribute("aria-expanded", String(!settingsPanel.hidden));
    requestDraw();
  };
  followButton.onclick = followLive;
}

function tooltipRows(hit: Hit): { title: string; dot: string; tag: string | null; rows: [string, string, string?][] } {
  if (hit.kind === "bubble") {
    const b = hit.shape.bubble;
    const rows: [string, string, string?][] = [
      ["Notional", usd(b.notional)],
      ["VWAP", price(b.vwap, priceDecimals)],
      ["Size", volume(b.size)],
      ["Trades", String(b.trades)],
      ["Price range", `${price(b.low, priceDecimals)} – ${price(b.high, priceDecimals)}`],
      ["Time", preciseClockUtc(b.start_us)],
    ];
    if (b.end_us > b.start_us) rows.push(["Duration", `${((b.end_us - b.start_us) / 1000).toFixed(0)} ms`]);
    return {
      title: `Taker ${b.side} Bubble`,
      dot: b.side,
      tag: b.large ? "Large" : null,
      rows,
    };
  }
  if (hit.kind === "cell") {
    const low = Number(hit.cell.price);
    const rows: [string, string, string?][] = [
      ["Taker buy", volume(hit.cell.buy), "buy"],
      ["Taker sell", volume(hit.cell.sell), "sell"],
      ["Delta", signedVolume(String(Number(hit.cell.buy) - Number(hit.cell.sell)))],
    ];
    if (hit.cell.imbalance) {
      rows.push(["Imbalance", hit.cell.imbalance, hit.cell.imbalance === "both" ? "" : hit.cell.imbalance]);
    }
    return {
      title: `${price(low, priceDecimals)} – ${price(low + cam.step, priceDecimals)}`,
      dot: "",
      tag: null,
      rows,
    };
  }
  if (hit.kind === "ladder") {
    const low = Number(hit.row.price);
    return {
      title: `${price(low, priceDecimals)} – ${price(low + cam.step, priceDecimals)}`,
      dot: "",
      tag: null,
      rows: [
        ["Resting bids", volume(hit.row.bid), "buy"],
        ["Resting asks", volume(hit.row.ask), "sell"],
      ],
    };
  }
  if (hit.kind === "bar") {
    const bar = hit.column.bar;
    return {
      title: `${clockUtc(bar.start_us)}–${clockUtc(bar.end_us)} UTC`,
      dot: "",
      tag: hit.column.incomplete ? "Incomplete" : null,
      rows: [
        ["Delta", signedVolume(bar.delta), Number(bar.delta) >= 0 ? "buy" : "sell"],
        ["Volume", volume(bar.total)],
        ["Trades", String(bar.trades)],
        ["CVD", signedVolume(bar.cvd)],
      ],
    };
  }
  const bar = hit.point.column.bar;
  return {
    title: "CVD",
    dot: "cvd",
    tag: hit.point.complete ? null : "Incomplete",
    rows: [
      ["Value", signedVolume(bar.cvd)],
      ["Bar delta", signedVolume(bar.delta)],
      ["At", `${clockUtc(bar.end_us)} UTC`],
    ],
  };
}

function updateTooltip(): void {
  const hit = pointer && scene ? hitTest(scene, pointer.x, pointer.y) : null;
  if (!hit || !pointer) {
    tooltip.hidden = true;
    return;
  }
  const content = tooltipRows(hit);
  const heading = document.createElement("h3");
  if (content.dot) {
    const dot = document.createElement("span");
    dot.className = `dot ${content.dot}`;
    heading.append(dot);
  }
  heading.append(content.title);
  if (content.tag) {
    const tag = document.createElement("span");
    tag.className = "tag";
    tag.textContent = content.tag;
    heading.append(tag);
  }
  const list = document.createElement("dl");
  for (const [label, value, tone] of content.rows) {
    const dt = document.createElement("dt");
    dt.textContent = label;
    const dd = document.createElement("dd");
    dd.textContent = value;
    if (tone) dd.className = tone;
    list.append(dt, dd);
  }
  tooltip.replaceChildren(heading, list);
  tooltip.hidden = false;
  const area = canvas.getBoundingClientRect();
  const box = tooltip.getBoundingClientRect();
  const left = pointer.x + 16 + box.width > area.width ? pointer.x - 16 - box.width : pointer.x + 16;
  const top = Math.min(Math.max(8, pointer.y + 16), area.height - box.height - 8);
  tooltip.style.left = `${Math.max(8, left)}px`;
  tooltip.style.top = `${top}px`;
}

type Drag = { mode: "pan" | "price"; x: number; y: number; cam: Camera };
let drag: Drag | null = null;

function local(event: PointerEvent | WheelEvent | MouseEvent): { x: number; y: number } {
  const rect = canvas.getBoundingClientRect();
  return { x: event.clientX - rect.left, y: event.clientY - rect.top };
}

function settleFollowing(): void {
  const newest = data.newestEnd();
  following = newest !== null && cam.rightEdgeUs >= newest;
}

canvas.addEventListener("pointerdown", (event) => {
  const at = local(event);
  const plotWidth = panes(layout()).plotWidth;
  canvas.setPointerCapture(event.pointerId);
  drag = { mode: at.x > plotWidth ? "price" : "pan", x: at.x, y: at.y, cam };
  canvas.classList.add(drag.mode === "pan" ? "dragging" : "axis");
});

canvas.addEventListener("pointermove", (event) => {
  const at = local(event);
  if (drag) {
    if (drag.mode === "pan") {
      cam = pan(drag.cam, at.x - drag.x, at.y - drag.y);
      settleFollowing();
    } else {
      cam = zoomPrice(drag.cam, mainHeight() / 2, Math.exp(-(at.y - drag.y) / 140));
    }
    pointer = null;
  } else {
    pointer = at;
  }
  requestDraw();
});

function endDrag(): void {
  drag = null;
  canvas.classList.remove("dragging", "axis");
}

canvas.addEventListener("pointerup", endDrag);
canvas.addEventListener("pointercancel", endDrag);
canvas.addEventListener("pointerleave", () => {
  pointer = null;
  requestDraw();
});
canvas.addEventListener("dblclick", followLive);
canvas.addEventListener(
  "wheel",
  (event) => {
    event.preventDefault();
    const at = local(event);
    const delta = event.deltaY || event.deltaX;
    const factor = delta > 0 ? 1 / WHEEL_ZOOM : WHEEL_ZOOM;
    const plotWidth = panes(layout()).plotWidth;
    if (event.ctrlKey || event.metaKey) {
      cam = zoomPrice(cam, Math.min(at.y, mainHeight()), factor);
    } else if (event.shiftKey) {
      cam = pan(cam, -delta, 0);
      settleFollowing();
    } else {
      cam = zoomTime(cam, plotWidth, following ? plotWidth : Math.min(at.x, plotWidth), factor);
      if (!following) settleFollowing();
    }
    requestDraw();
  },
  { passive: false },
);

async function start(): Promise<void> {
  document.documentElement.dataset.theme = theme;
  buildControls();
  try {
    markets = await fetchMarkets();
  } catch (error) {
    showNotice(`Gateway unavailable: ${(error as Error).message}`);
    setStatus("Offline", "offline");
    return;
  }
  for (const m of markets) option(marketSelect, m.id, `${m.native_name} · ${m.venue} ${m.market_type}`);
  if (!markets.some((m) => m.id === view.market)) view = { ...view, market: markets[0]?.id ?? null };
  await reload();
}

void start();
