import { describe, expect, it } from "vitest";
import { PEAK_COUNT, peaksOf, waveformBars } from "./waveform";

describe("waveforms", () => {
  it("draws stored peaks as bars between a minimum and full height", () => {
    const bars = waveformBars(new Uint8Array([0, 0, 255, 255, 128, 128, 0, 0]), 4);
    expect(bars).toEqual([0.12, 1, 128 / 255, 0.12]);
  });

  it("draws a flat line for a message without a waveform", () => {
    expect(waveformBars(new Uint8Array(), 5)).toEqual([0.12, 0.12, 0.12, 0.12, 0.12]);
  });

  it("takes the loudest sample of each stretch when there are more samples than bars", () => {
    const [first, second] = waveformBars([10, 200, 30, 40], 2);
    expect(first).toBeCloseTo(200 / 255);
    expect(second).toBeCloseTo(40 / 255);
  });

  it("reduces a recording's loudness to the peaks a message carries, scaled to its loudest moment", () => {
    const levels = Array.from({ length: 640 }, (_, index) => (index < 320 ? 0.1 : 0.4));
    const peaks = peaksOf(levels);
    expect(peaks).toHaveLength(PEAK_COUNT);
    expect(peaks[0]).toBe(64);
    expect(peaks[PEAK_COUNT - 1]).toBe(255);
  });

  it("gives a silent waveform for an empty recording", () => {
    expect([...peaksOf([], 4)]).toEqual([0, 0, 0, 0]);
  });
});
