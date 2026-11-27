import { school } from "@tokuly/api";
import * as timer from "@tokuly/core/school";

/**
 * The server's timers on this phone's clock. The arithmetic is
 * `@tokuly/core/school`; this keeps how far the phone's clock is from the
 * server's, learned from every snapshot that arrives.
 */

let offsetMs = 0;

/** Notes the server's clock from a snapshot that just arrived. */
export function syncClock(snapshot: school.TimerSnapshot | undefined): void {
  offsetMs = timer.clockOffset(snapshot, Date.now()) ?? offsetMs;
}

export function timerAt(snapshot: school.TimerSnapshot | undefined, nowMs: number) {
  return timer.timerAt(snapshot, nowMs, offsetMs);
}

/**
 * When the phase a running pomodoro is in at `nowMs` ends, on this phone's
 * clock; `undefined` for a stopwatch or a timer that is not running.
 */
export function phaseEndAt(
  snapshot: school.TimerSnapshot | undefined,
  nowMs: number,
): number | undefined {
  const view = timerAt(snapshot, nowMs);
  if (view.mode !== school.StudyMode.POMODORO || view.state !== school.TimerState.RUNNING)
    return undefined;
  return nowMs + view.remainingMs;
}

export function studiedSecondsAt(session: school.StudySession, nowMs: number): number {
  return timer.studiedSecondsAt(session, nowMs, offsetMs);
}
