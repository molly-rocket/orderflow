import { defineConfig } from "@playwright/test";

// Browser smoke tests against a running gateway (default: the local live stack).
// Uses the installed Chrome, so no browser download is needed.
export default defineConfig({
  testDir: "e2e",
  timeout: 120_000,
  use: {
    baseURL: process.env.ORDERFLOW_URL ?? "http://127.0.0.1:8080",
    channel: "chrome",
    viewport: { width: 1400, height: 800 },
  },
});
