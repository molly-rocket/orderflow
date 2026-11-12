import { describe, expect, it } from "vitest";
import { clockUtc, dayLabelUtc, decimals, preciseClockUtc, price, signedVolume, usd, volume } from "./format";

describe("display formatting", () => {
  it("shortens volumes without inventing precision", () => {
    expect(volume("0.086")).toBe("0.086");
    expect(volume("0.0004")).toBe("0");
    expect(volume("12.5")).toBe("12.5");
    expect(volume("150.7")).toBe("151");
    expect(volume("1074.077")).toBe("1.1k");
    expect(volume("3400000")).toBe("3.4M");
    expect(signedVolume("6")).toBe("+6");
    expect(signedVolume("-2.25")).toBe("-2.25");
    expect(signedVolume("0")).toBe("0");
  });

  it("formats USD notionals and prices", () => {
    expect(usd("404")).toBe("$404");
    expect(usd("49.5")).toBe("$49.5");
    expect(usd("125000")).toBe("$125k");
    expect(usd("1234567")).toBe("$1.23M");
    expect(usd("2500000000")).toBe("$2.5B");
    expect(price("74483.4", 1)).toBe("74,483.4");
    expect(price("101", 2)).toBe("101.00");
  });

  it("counts decimals and formats UTC times", () => {
    expect(decimals("0.1")).toBe(1);
    expect(decimals("0.10")).toBe(2);
    expect(decimals("5")).toBe(0);
    expect(clockUtc(1_794_179_700_000_000)).toBe("23:15");
    expect(preciseClockUtc(1_794_179_700_123_000)).toBe("23:15:00.123 UTC");
    expect(dayLabelUtc(1_794_096_000_000_000)).toBe("Nov 8");
  });
});
