import { shorts } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import { reportStay, StayMeter } from "./stay";

describe("StayMeter", () => {
  it("counts only the time playback was moving", () => {
    const meter = new StayMeter();
    meter.play(1_000);
    meter.pause(3_000); // paused by a tap
    meter.play(10_000);
    expect(meter.read(10_500)).toEqual({ watchedMs: 2_500, completions: 0 });
  });

  it("counts replays and every time the end was reached", () => {
    const meter = new StayMeter();
    meter.play(0);
    meter.complete(6_000);
    meter.play(6_050);
    meter.complete(12_050);
    meter.play(12_100);
    meter.pause(13_100);
    expect(meter.read(20_000)).toEqual({ watchedMs: 13_000, completions: 2 });
  });

  it("is not thrown off by repeated or stray events", () => {
    const meter = new StayMeter();
    meter.pause(5);
    meter.play(10);
    meter.play(500);
    meter.pause(1_010);
    meter.pause(4_000);
    expect(meter.read(9_000)).toEqual({ watchedMs: 1_000, completions: 0 });
  });
});

describe("reportStay", () => {
  it("sends the stays of one moment together", async () => {
    const calls: { shortId: string; watchedMs: bigint }[][] = [];
    const send = async (reports: { shortId: string; watchedMs: bigint }[]) => {
      calls.push(reports);
    };
    const source = shorts.PlaybackSource.FOR_YOU;
    reportStay("a", source, { watchedMs: 1_200, completions: 0 }, send);
    reportStay("b", source, { watchedMs: 300, completions: 1 }, send);
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(calls).toHaveLength(1);
    expect(calls[0]?.map((report) => [report.shortId, report.watchedMs])).toEqual([
      ["a", 1_200n],
      ["b", 300n],
    ]);
  });
});
