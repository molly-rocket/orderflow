import { describe, expect, it } from "vitest";
import { clockUtc, decimals, signedVolume, volume } from "./format";

describe("display formatting", () => {
  it("shortens volumes without inventing precision", () => {
    expect(volume("0.086")).toBe("0.086");
    expect(volume("0.0004")).toBe("0");
    expect(volume("12.5")).toBe("12.5");
    expect(volume("12.50")).toBe("12.5");
    expect(volume("150.7")).toBe("151");
    expect(volume("1074.077")).toBe("1.1k");
    expect(volume("3400000")).toBe("3.4M");
    expect(signedVolume("6")).toBe("+6");
    expect(signedVolume("-2.25")).toBe("-2.25");
    expect(signedVolume("0")).toBe("0");
  });

  it("counts decimals and formats UTC clock times", () => {
    expect(decimals("0.1")).toBe(1);
    expect(decimals("0.10")).toBe(2);
    expect(decimals("5")).toBe(0);
    // 1794179700 s = 2026-11-08 23:15:00 UTC (checked with Python datetime).
    expect(clockUtc(1_794_179_700_000_000)).toBe("23:15");
  });
});
