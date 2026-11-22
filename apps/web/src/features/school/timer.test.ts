import { create, type MessageInitShape, school } from "@tokuly/api";
import { timestampFromMs } from "@bufbuild/protobuf/wkt";
import { beforeEach, describe, expect, it } from "vitest";
import { studiedSecondsAt, syncClock, timerAt } from "./timer";

const T0 = Date.UTC(2026, 10, 22, 9, 0, 0);

function snapshot(init: MessageInitShape<typeof school.TimerSnapshotSchema>, serverMs = T0): school.TimerSnapshot {
  return create(school.TimerSnapshotSchema, { serverTime: timestampFromMs(serverMs), ...init });
}

const pomodoro = { workMinutes: 25, breakMinutes: 5, longBreakMinutes: 15, rounds: 2 };

describe("timerAt", () => {
  // The client's clock and the server's agree unless a test says otherwise.
  beforeEach(() => syncClock(snapshot({}), T0));

  it("counts a running stopwatch from the snapshot's server time", () => {
    const timer = snapshot({ mode: school.StudyMode.STOPWATCH, state: school.TimerState.RUNNING, elapsedSeconds: 60 });
    const view = timerAt(timer, T0 + 90_000);
    expect(view.elapsedMs).toBe(150_000);
    expect(view.workSinceMs).toBe(90_000);
    expect(view.phase).toBe(school.TimerPhase.WORK);
  });

  it("stands still while paused, however much time passes", () => {
    const timer = snapshot({ mode: school.StudyMode.STOPWATCH, state: school.TimerState.PAUSED, elapsedSeconds: 600 });
    expect(timerAt(timer, T0 + 3_600_000).elapsedMs).toBe(600_000);
    expect(timerAt(timer, T0 + 3_600_000).workSinceMs).toBe(0);
  });

  it("corrects for a client clock that runs ahead of the server", () => {
    const timer = snapshot({ mode: school.StudyMode.STOPWATCH, state: school.TimerState.RUNNING, elapsedSeconds: 0 });
    // The snapshot arrives when the client's clock says ten minutes later than the server's.
    syncClock(timer, T0 + 600_000);
    expect(timerAt(timer, T0 + 600_000 + 5_000).elapsedMs).toBe(5_000);
  });

  it("moves a running pomodoro through work, break, work and the long break by itself", () => {
    const timer = snapshot({ mode: school.StudyMode.POMODORO, pomodoro, state: school.TimerState.RUNNING, phase: school.TimerPhase.WORK, round: 1, elapsedSeconds: 0, phaseEndsAt: timestampFromMs(T0 + 25 * 60_000) });
    const at = (minutes: number) => timerAt(timer, T0 + minutes * 60_000);

    expect(at(10)).toMatchObject({ phase: school.TimerPhase.WORK, round: 1, remainingMs: 15 * 60_000, phaseMs: 25 * 60_000 });
    expect(at(27)).toMatchObject({ phase: school.TimerPhase.BREAK, round: 2, remainingMs: 3 * 60_000 });
    expect(at(40)).toMatchObject({ phase: school.TimerPhase.WORK, round: 2, remainingMs: 15 * 60_000 });
    // After the last round of the cycle comes the long break, then round 3 begins a new cycle.
    expect(at(60)).toMatchObject({ phase: school.TimerPhase.LONG_BREAK, round: 3, remainingMs: 10 * 60_000 });
    expect(at(71)).toMatchObject({ phase: school.TimerPhase.WORK, round: 3 });
  });

  it("counts only work phases as study time", () => {
    const timer = snapshot({ mode: school.StudyMode.POMODORO, pomodoro, state: school.TimerState.RUNNING, phase: school.TimerPhase.WORK, round: 1, elapsedSeconds: 0 });
    // 25 work + 5 break + 10 of the second work phase.
    expect(timerAt(timer, T0 + 40 * 60_000).workSinceMs).toBe(35 * 60_000);
    const session = { state: school.SessionState.RUNNING, studiedSeconds: 120, timer };
    expect(studiedSecondsAt(session, T0 + 27 * 60_000)).toBe(120 + 25 * 60);
  });

  it("uses the phase's end to place a snapshot taken mid-second", () => {
    // 100.6 s into the first work phase; `elapsed_seconds` only says 100.
    const timer = snapshot({ mode: school.StudyMode.POMODORO, pomodoro, state: school.TimerState.RUNNING, phase: school.TimerPhase.WORK, round: 1, elapsedSeconds: 100, phaseEndsAt: timestampFromMs(T0 + 25 * 60_000 - 100_600) });
    expect(timerAt(timer, T0).remainingMs).toBe(25 * 60_000 - 100_600);
  });

  it("reports the counted time of a finished or paused session as it is", () => {
    const timer = snapshot({ mode: school.StudyMode.STOPWATCH, state: school.TimerState.RUNNING });
    expect(studiedSecondsAt({ state: school.SessionState.PAUSED, studiedSeconds: 300, timer }, T0 + 60_000)).toBe(300);
    expect(studiedSecondsAt({ state: school.SessionState.ENDED, studiedSeconds: 300, timer }, T0 + 60_000)).toBe(300);
  });

  it("shows an idle timer with its first phase ahead of it", () => {
    const timer = snapshot({ mode: school.StudyMode.POMODORO, pomodoro, state: school.TimerState.IDLE });
    expect(timerAt(timer, T0 + 60_000)).toMatchObject({ state: school.TimerState.IDLE, phase: school.TimerPhase.WORK, round: 1, remainingMs: 25 * 60_000 });
  });
});
