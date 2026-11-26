import { create } from "@bufbuild/protobuf";
import { live } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import { clampClipRange, fillAlertTemplate, maskSecret, maskUrl, tierStyle } from "./model";

const tier = (level: number, minCoins: bigint, gradientColors: string[] = []) =>
  create(live.GiftTierSchema, { level, minCoins, backgroundColor: "#111111", gradientColors });

describe("gift tiers", () => {
  it("draws the top tier as a gradient", () => {
    expect(tierStyle(tier(12, 1n, ["#ff0000", "#00ff00"])).background).toBe(
      "linear-gradient(100deg, #ff0000, #00ff00)",
    );
    expect(tierStyle(tier(1, 1n)).background).toBe("#111111");
  });
});

describe("clip range", () => {
  it("never runs past a minute", () => {
    expect(clampClipRange(10, 200, 600, "end")).toEqual({ start: 140, end: 200 });
    expect(clampClipRange(10, 200, 600, "start")).toEqual({ start: 10, end: 70 });
  });

  it("stays inside the source and never collapses", () => {
    expect(clampClipRange(-5, 3, 20, "start")).toEqual({ start: 0, end: 3 });
    expect(clampClipRange(15, 99, 20, "end")).toEqual({ start: 15, end: 20 });
    expect(clampClipRange(8, 8, 20, "start")).toEqual({ start: 8, end: 8.1 });
    expect(clampClipRange(8, 2, 20, "end")).toEqual({ start: 1.9, end: 2 });
  });
});

describe("secrets", () => {
  it("masks without leaking the length", () => {
    expect(maskSecret("abc")).toBe(maskSecret("a-much-longer-secret"));
    expect(maskSecret("")).toBe("");
  });

  it("keeps the host of a URL readable and hides the rest", () => {
    const masked = maskUrl("rtmp://live.example:1935/live/abc?token=SECRET");
    expect(masked.startsWith("rtmp://live.example:1935/")).toBe(true);
    expect(masked).not.toContain("SECRET");
    expect(masked).not.toContain("abc");
  });
});

describe("alert text", () => {
  it("fills the placeholders", () => {
    expect(
      fillAlertTemplate("{name} が {coins} コイン! {message} ({count})", {
        name: "ミナ",
        coins: "500",
        message: "がんばれ",
        count: "3",
      }),
    ).toBe("ミナ が 500 コイン! がんばれ (3)");
  });
});
