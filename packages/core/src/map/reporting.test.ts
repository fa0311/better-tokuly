import { toMillis } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import { type LastReport, locationSample, type OwnPosition, reportDecision } from "./reporting";

const NOW = Date.UTC(2026, 10, 25, 9, 0, 0);
const CADENCE = { movingSeconds: 5, stationarySeconds: 60 };

const fix = (overrides: Partial<OwnPosition> = {}): OwnPosition => ({
  lat: 35.66,
  lng: 139.7,
  accuracyMeters: 10,
  speedMps: 0,
  headingDegrees: null,
  recordedAt: NOW,
  ...overrides,
});
const sent = (secondsAgo: number, position = fix({ recordedAt: NOW - secondsAgo * 1000 })) =>
  ({ at: NOW - secondsAgo * 1000, position }) satisfies LastReport;

describe("reportDecision", () => {
  it("sends the first fix at once", () => {
    expect(reportDecision(fix(), null, CADENCE, NOW)).toBe("send");
  });

  it("waits for the stationary interval while still", () => {
    expect(reportDecision(fix(), sent(30), CADENCE, NOW)).toBe("wait");
    expect(reportDecision(fix(), sent(61), CADENCE, NOW)).toBe("send");
  });

  it("goes at the moving pace when the device reports speed", () => {
    expect(reportDecision(fix({ speedMps: 3 }), sent(6), CADENCE, NOW)).toBe("send");
    expect(reportDecision(fix({ speedMps: 3 }), sent(2), CADENCE, NOW)).toBe("wait");
  });

  it("counts a fix far from the last one as moving, without speed", () => {
    const moved = fix({ lat: 35.661, speedMps: null });
    expect(reportDecision(moved, sent(6), CADENCE, NOW)).toBe("send");
  });

  it("does not count a move inside the fix's own accuracy", () => {
    const jitter = fix({ lat: 35.6601, accuracyMeters: 50, speedMps: null });
    expect(reportDecision(jitter, sent(6), CADENCE, NOW)).toBe("wait");
  });

  it("drops the fix already sent, even when forced", () => {
    const last = sent(120, fix());
    expect(reportDecision(fix(), last, CADENCE, NOW, true)).toBe("same");
  });

  it("sends a forced new fix before its turn", () => {
    expect(reportDecision(fix(), sent(1), CADENCE, NOW, true)).toBe("send");
  });
});

describe("locationSample", () => {
  it("keeps the moment the fix was taken and leaves out what the platform could not tell", () => {
    const sample = locationSample(fix({ speedMps: Number.NaN, headingDegrees: -1 }), {
      batteryPercent: 41.6,
      charging: false,
      foreground: false,
    });
    expect(toMillis(sample.recordedAt)).toBe(NOW);
    expect(sample.speedMps).toBeUndefined();
    expect(sample.headingDegrees).toBeUndefined();
    expect(sample.batteryPercent).toBe(42);
    expect(sample.foreground).toBe(false);
  });
});
