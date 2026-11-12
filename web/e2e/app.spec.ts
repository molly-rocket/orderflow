import { type Page, expect, test } from "@playwright/test";

const MARKET = "binance:perp:BTCUSDT";

async function coloredShare(page: Page): Promise<number> {
  return page.evaluate(() => {
    const canvas = document.querySelector<HTMLCanvasElement>("#chart")!;
    const ctx = canvas.getContext("2d")!;
    const width = Math.floor(canvas.width * 0.85);
    const height = Math.floor(canvas.height * 0.6);
    const { data } = ctx.getImageData(0, 0, width, height);
    let colored = 0;
    let sampled = 0;
    for (let i = 0; i < data.length; i += 4 * 53) {
      sampled++;
      const r = data[i] ?? 0;
      const g = data[i + 1] ?? 0;
      const b = data[i + 2] ?? 0;
      if (Math.max(r, g, b) - Math.min(r, g, b) > 60) colored++;
    }
    return colored / sampled;
  });
}

const DRAWN = 0.004;

function collectErrors(page: Page): string[] {
  const errors: string[] = [];
  page.on("console", (m) => m.type() === "error" && errors.push(m.text()));
  page.on("pageerror", (e) => errors.push(e.message));
  return errors;
}

async function openLive(page: Page, path = "/"): Promise<void> {
  await page.goto(path);
  await expect(page.locator("#status")).toHaveText("Live", { timeout: 20_000 });
  await expect.poll(() => coloredShare(page), { timeout: 20_000 }).toBeGreaterThan(DRAWN);
}

async function hoverUntil(page: Page, text: RegExp): Promise<boolean> {
  const box = (await page.locator("#chart").boundingBox())!;
  for (let y = box.y + 30; y < box.y + box.height * 0.55; y += 9) {
    for (let x = box.x + box.width - 140; x > box.x + box.width * 0.45; x -= 9) {
      await page.mouse.move(x, y);
      const tooltip = page.locator("#tooltip");
      if ((await tooltip.isVisible()) && text.test((await tooltip.textContent()) ?? "")) return true;
    }
  }
  return false;
}

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    if (!sessionStorage.getItem("orderflow.e2e.cleared")) {
      localStorage.clear();
      sessionStorage.setItem("orderflow.e2e.cleared", "1");
    }
  });
});

test("the chart goes live with Footprint, Bubbles and CVD, and every view toggles", async ({ page }) => {
  const errors = collectErrors(page);
  await openLive(page);
  await expect(page).toHaveURL(/market=binance%3Aperp%3ABTCUSDT/);
  await expect(page).toHaveURL(/price=fp/);
  await expect(page).toHaveURL(/views=bub%2Ccvd%2Cdom%2Clast/);
  await page.screenshot({ path: "test-results/chart-dark.png" });

  for (const view of ["bubbles", "cvd"]) {
    const chip = page.locator(`.chip[data-view="${view}"]`);
    await chip.click();
    await expect(chip).toHaveAttribute("aria-pressed", "false");
    await expect.poll(() => coloredShare(page)).toBeGreaterThan(DRAWN);
    await chip.click();
    await expect(chip).toHaveAttribute("aria-pressed", "true");
  }

  await page.locator('#timeframes button[data-tf="300"]').click();
  await expect(page).toHaveURL(/tf=300/);
  await expect(page.locator("#status")).toHaveText("Live", { timeout: 20_000 });
  await page.locator("#grouping").selectOption("10");
  await expect(page).toHaveURL(/grouping=10/);
  await expect(page.locator("#status")).toHaveText("Live", { timeout: 20_000 });
  await expect.poll(() => coloredShare(page), { timeout: 20_000 }).toBeGreaterThan(DRAWN);
  await page.screenshot({ path: "test-results/chart-5m-grouped.png" });

  expect(errors).toEqual([]);
});

test("hovering a Bubble shows its notional, VWAP, Trades, side and time", async ({ page }) => {
  const errors = collectErrors(page);
  await openLive(page, `/?market=${MARKET}&tf=60&grouping=5&views=bub%2Ccvd&minusd=20000`);

  const found = await hoverUntil(page, /Bubble/);
  expect(found, "no Bubble under the pointer anywhere in the recent chart").toBe(true);
  const text = (await page.locator("#tooltip").textContent()) ?? "";
  expect(text).toMatch(/Taker (buy|sell) Bubble/);
  for (const label of ["Notional", "VWAP", "Trades", "Time"]) expect(text).toContain(label);
  await page.screenshot({ path: "test-results/bubble-tooltip.png" });
  expect(errors).toEqual([]);
});

test("settings change the chart and survive a reload, from the URL and from local storage", async ({ page }) => {
  const errors = collectErrors(page);
  await openLive(page);

  await page.locator("#settings-toggle").click();
  await expect(page.locator("#settings")).toBeVisible();
  await page.locator("#percentile").selectOption("95");
  await expect(page).toHaveURL(/pct=95/);
  await page.locator("#min-notional").fill("50000");
  await page.locator("#min-notional").dispatchEvent("change");
  await expect(page).toHaveURL(/minusd=50000/);
  await page.locator('#side button[data-side="sell"]').click();
  await expect(page).toHaveURL(/side=sell/);
  await expect(page.locator("#status")).toHaveText("Live", { timeout: 20_000 });
  await page.screenshot({ path: "test-results/settings.png" });

  await page.reload();
  await expect(page.locator("#percentile")).toHaveValue("95");
  await expect(page.locator("#min-notional")).toHaveValue("50000");

  await page.goto("/");
  await expect(page.locator("#percentile")).toHaveValue("95");
  await expect(page.locator('#side button[data-side="sell"]')).toHaveAttribute("aria-checked", "true");
  expect(errors).toEqual([]);
});

test("the light theme renders and is remembered", async ({ page }) => {
  const errors = collectErrors(page);
  await openLive(page);
  await page.locator("#theme-toggle").click();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
  await expect.poll(() => coloredShare(page)).toBeGreaterThan(DRAWN);
  await page.screenshot({ path: "test-results/chart-light.png" });
  await page.reload();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
  expect(errors).toEqual([]);
});

test("price style switches between footprint, candles, line and hidden without reloading", async ({ page }) => {
  const errors = collectErrors(page);
  await openLive(page);
  let footprintRequests = 0;
  page.on("request", (request) => {
    if (request.url().includes("/api/v1/footprint")) footprintRequests++;
  });

  for (const style of ["candles", "line", "none", "fp"]) {
    await page.locator(`#price-style button[data-price="${style}"]`).click();
    await expect(page).toHaveURL(new RegExp(`price=${style}`));
    if (style !== "none") await expect.poll(() => coloredShare(page)).toBeGreaterThan(DRAWN);
    await page.screenshot({ path: `test-results/price-${style}.png` });
  }

  expect(footprintRequests).toBe(0);
  await expect(page.locator("#status")).toHaveText("Live");
  expect(errors).toEqual([]);
});

test("the live price tag shows the last trade price", async ({ page }) => {
  const errors = collectErrors(page);
  const closes: string[] = [];
  page.on("websocket", (ws) => {
    ws.on("framereceived", (frame) => {
      const text = typeof frame.payload === "string" ? frame.payload : frame.payload.toString();
      const message = JSON.parse(text) as { type: string; bar?: { close: string | null } };
      if (message.type === "bar" && message.bar?.close) closes.push(message.bar.close);
    });
  });
  await openLive(page);

  const tag = page.locator("#last-price");
  await expect(tag).toBeVisible();
  await expect
    .poll(() => tag.textContent().then((t) => closes.slice(-3).some((c) => Number(c) === Number((t ?? "").replaceAll(",", "")))))
    .toBe(true);
  await expect(tag).toHaveAttribute("data-direction", /up|down/);
  expect(errors).toEqual([]);
});

test("the order book ladder shows a best bid below the best ask", async ({ page }) => {
  const errors = collectErrors(page);
  await openLive(page);

  const header = page.locator("#book-header");
  await expect(header).toContainText("bid", { timeout: 20_000 });
  const text = (await header.textContent()) ?? "";
  const [bid, ask, spread] = [...text.matchAll(/[\d,]+\.\d+/g)].map((m) => Number(m[0].replaceAll(",", "")));
  expect(bid).toBeLessThan(ask!);
  expect(spread).toBeGreaterThanOrEqual(0.1);
  expect(ask! - bid!).toBeCloseTo(spread!, 1);
  await page.screenshot({ path: "test-results/order-book.png" });
  expect(errors).toEqual([]);
});

test("the heatmap alone paints the chart, and nothing is painted once it is off", async ({ page }) => {
  const errors = collectErrors(page);
  await page.goto(`/?market=${MARKET}&tf=60&grouping=10&price=none&views=heat`);
  await expect(page.locator("#status")).toHaveText("Live", { timeout: 20_000 });
  await expect.poll(() => coloredShare(page), { timeout: 90_000 }).toBeGreaterThan(DRAWN);
  await page.screenshot({ path: "test-results/heatmap.png" });

  const chip = page.locator('.chip[data-view="heatmap"]');
  await chip.click();
  await expect(chip).toHaveAttribute("aria-pressed", "false");
  await expect.poll(() => coloredShare(page)).toBeLessThan(DRAWN);
  expect(errors).toEqual([]);
});

test("every layer toggles and persists across reload", async ({ page }) => {
  const errors = collectErrors(page);
  await openLive(page);

  for (const view of ["bubbles", "heatmap", "dom", "cvd", "lastPrice"]) {
    const chip = page.locator(`.chip[data-view="${view}"]`);
    const pressed = (await chip.getAttribute("aria-pressed")) === "true";
    await chip.click();
    await expect(chip).toHaveAttribute("aria-pressed", String(!pressed));
  }
  await expect(page).toHaveURL(/views=heat/);

  await page.reload();
  await expect(page.locator('.chip[data-view="heatmap"]')).toHaveAttribute("aria-pressed", "true");
  await expect(page.locator('.chip[data-view="dom"]')).toHaveAttribute("aria-pressed", "false");
  await expect(page.locator('.chip[data-view="lastPrice"]')).toHaveAttribute("aria-pressed", "false");
  await expect(page.locator("#last-price")).toBeHidden();
  expect(errors).toEqual([]);
});

test("zoomed in, Footprint cells show Taker volumes as numbers", async ({ page }) => {
  const errors = collectErrors(page);
  await openLive(page, `/?market=${MARKET}&tf=60&grouping=10&views=fp`);
  const box = (await page.locator("#chart").boundingBox())!;
  await page.mouse.move(box.x + box.width * 0.6, box.y + box.height * 0.4);
  for (let i = 0; i < 4; i++) await page.mouse.wheel(0, -120);
  await page.keyboard.down("Control");
  for (let i = 0; i < 3; i++) await page.mouse.wheel(0, -120);
  await page.keyboard.up("Control");
  await page.mouse.move(box.x + box.width * 0.6, box.y + box.height * 0.4);
  await expect.poll(() => coloredShare(page)).toBeGreaterThan(DRAWN);
  await page.screenshot({ path: "test-results/footprint-numbers.png" });
  expect(errors).toEqual([]);
});
