import { defineConfig } from "vitest/config";

// In development the gateway runs on :8080; proxy the API (including the live WebSocket).
export default defineConfig({
  server: {
    port: 5173,
    proxy: {
      "/api": { target: "http://127.0.0.1:8080", ws: true },
    },
  },
  test: {
    environment: "node",
    include: ["src/**/*.test.ts"],
  },
});
