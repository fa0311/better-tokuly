import { describe, expect, it } from "vitest";
import { StayMeter } from "./stay";

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
