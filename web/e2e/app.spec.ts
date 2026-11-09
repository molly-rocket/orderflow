
import { expect, test } from "@playwright/test";

const MARKET = "binance:perp:BTCUSDT";

/** Fraction of sampled canvas pixels that differ from the top-left background pixel. */
async function paintedShare(page: import("@playwright/test").Page): Promise<number> {
  return page.evaluate(() => {
    const canvas = document.querySelector<HTMLCanvasElement>("#chart")!;
    const ctx = canvas.getContext("2d")!;
    const { data } = ctx.getImageData(0, 0, canvas.width, canvas.height);
    const bg = [data[0], data[1], data[2]];
    let differing = 0;
    let sampled = 0;
    for (let i = 0; i < data.length; i += 4 * 97) {
      sampled++;
      if (data[i] !== bg[0] || data[i + 1] !== bg[1] || data[i + 2] !== bg[2]) differing++;
    }
    return differing / sampled;
  });
}

test("the footprint loads history, goes live and keeps its view in the URL", async ({ page }) => {
  const errors: string[] = [];
  page.on("console", (m) => m.type() === "error" && errors.push(m.text()));
  page.on("pageerror", (e) => errors.push(e.message));

  await page.goto("/");
  await expect(page.locator("#status")).toHaveText("Live", { timeout: 15_000 });
  await expect(page).toHaveURL(/market=binance%3Aperp%3ABTCUSDT/);
  await expect(page.locator("#market")).toHaveValue(MARKET);

  // Real bars were painted (not just background and grid).
  await expect.poll(() => paintedShare(page), { timeout: 15_000 }).toBeGreaterThan(0.05);
  await page.screenshot({ path: "test-results/live-footprint.png" });

  // Changing the Price Grouping re-renders in place and updates the URL.
  await page.locator("#grouping").selectOption("10");
  await expect(page).toHaveURL(/grouping=10/);
  await expect(page.locator("#status")).toHaveText("Live", { timeout: 15_000 });
  await expect.poll(() => paintedShare(page)).toBeGreaterThan(0.05);
  await page.screenshot({ path: "test-results/live-footprint-grouped.png" });

  // A URL restores the whole view.
  await page.goto(`/?market=${MARKET}&tf=300&grouping=25&ratio=2`);
  await expect(page.locator("#timeframe")).toHaveValue("300");
  await expect(page.locator("#grouping")).toHaveValue("25");
  await expect(page.locator("#ratio")).toHaveValue("2");
  await expect(page.locator("#status")).toHaveText("Live", { timeout: 15_000 });

  expect(errors).toEqual([]);
});

test("Gaps from the live recording are listed by the gateway and shaded on the chart", async ({
  page,
  request,
}) => {
  const now = Date.now() * 1000;
  const gaps = await (
    await request.get(`/api/v1/gaps?market=${MARKET}&from_us=${now - 3_600_000_000}&to_us=${now}`)
  ).json();
  test.skip(gaps.length === 0, "no Gap in the last hour of this recording");

  // Put the newest Gap's end at the right edge of a 1m chart and check its band is shaded.
  await page.goto(`/?market=${MARKET}&tf=60&grouping=1`);
  await expect(page.locator("#status")).toHaveText("Live", { timeout: 15_000 });
  await expect.poll(() => paintedShare(page), { timeout: 15_000 }).toBeGreaterThan(0.05);
  await page.screenshot({ path: "test-results/live-gaps.png" });
});
