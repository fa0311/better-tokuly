import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { PlaybackReporter, type PlaybackReporterDeps } from "./playback";

function setup(startDelayMs = 0) {
  const calls = {
    start: 0,
    reports: [] as { sessionId: string; playing: boolean; ended: boolean }[],
  };
  let gone = false;
  const deps: PlaybackReporterDeps<number> = {
    startPlayback: async () => {
      calls.start += 1;
      await new Promise((resolve) => setTimeout(resolve, startDelayMs));
      return { sessionId: `session-${calls.start}`, reportIntervalSeconds: 10, viewCount: 1n };
    },
    reportPlayback: async (request) => {
      calls.reports.push({
        sessionId: request.sessionId,
        playing: request.playing,
        ended: request.ended,
      });
      if (gone) throw new Error("gone");
      return { viewCount: 2n };
    },
    position: () => 12,
    isPlaying: () => true,
    isSessionGone: (error) => error instanceof Error && error.message === "gone",
  };
  return {
    calls,
    reporter: new PlaybackReporter("video", 1, deps),
    expire: () => (gone = true),
    revive: () => (gone = false),
  };
}

describe("PlaybackReporter", () => {
  beforeEach(() => void vi.useFakeTimers());
  afterEach(() => void vi.useRealTimers());

  it("opens one session and reports at the server's cadence", async () => {
    const { calls, reporter } = setup();
    reporter.play();
    reporter.play();
    await vi.advanceTimersByTimeAsync(0);
    expect(calls.start).toBe(1);
    expect(reporter.sessionId).toBe("session-1");
    await vi.advanceTimersByTimeAsync(30_000);
    expect(calls.reports).toHaveLength(3);
    await reporter.report(true);
    expect(calls.reports.at(-1)).toEqual({ sessionId: "session-1", playing: false, ended: true });
  });

  it("stops for good when disposed", async () => {
    const { calls, reporter } = setup();
    reporter.play();
    await vi.advanceTimersByTimeAsync(10_000);
    reporter.dispose();
    await vi.advanceTimersByTimeAsync(60_000);
    expect(calls.reports).toHaveLength(1);
    expect(vi.getTimerCount()).toBe(0);
  });

  it("drops a session whose opening answers after the viewer left", async () => {
    const { calls, reporter } = setup(3_000);
    reporter.play();
    await vi.advanceTimersByTimeAsync(1_000);
    reporter.dispose();
    // The answer arrives now: nothing is kept, nothing is scheduled, nothing is ever reported.
    await vi.advanceTimersByTimeAsync(120_000);
    expect(calls.start).toBe(1);
    expect(reporter.sessionId).toBeNull();
    expect(calls.reports).toHaveLength(0);
    expect(vi.getTimerCount()).toBe(0);
    reporter.play();
    await vi.advanceTimersByTimeAsync(10_000);
    expect(calls.start).toBe(1);
  });

  it("opens a new session when the old one expired", async () => {
    const { calls, reporter, expire, revive } = setup();
    reporter.play();
    await vi.advanceTimersByTimeAsync(0);
    expire();
    await vi.advanceTimersByTimeAsync(10_000);
    expect(reporter.sessionId).toBeNull();
    revive();
    // The next tick finds the video playing and opens a new session; its answer lands a moment later.
    await vi.advanceTimersByTimeAsync(10_000);
    await vi.advanceTimersByTimeAsync(1);
    expect(calls.start).toBe(2);
    expect(reporter.sessionId).toBe("session-2");
    reporter.dispose();
  });
});
