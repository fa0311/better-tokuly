import type { school } from "@tokuly/api";
import * as timer from "@tokuly/core/school";

/**
 * The server's timers on this client's clock. The arithmetic is
 * `@tokuly/core/school`; this keeps how far the browser's clock is from the
 * server's, learned from every snapshot that arrives.
 */

let offsetMs = 0;

/** Notes the server's clock from a snapshot that just arrived. */
export function syncClock(
  snapshot: school.TimerSnapshot | undefined,
  receivedAtMs: number = Date.now(),
): void {
  offsetMs = timer.clockOffset(snapshot, receivedAtMs) ?? offsetMs;
}

/** The server's time now, as far as this client can tell. */
export function serverNow(nowMs: number = Date.now()): number {
  return nowMs + offsetMs;
}

/** Where a timer stands at `nowMs` (this client's clock). */
export function timerAt(snapshot: school.TimerSnapshot | undefined, nowMs: number = Date.now()) {
  return timer.timerAt(snapshot, nowMs, offsetMs);
}

/** A session's counted study time at `nowMs`, in seconds. */
export function studiedSecondsAt(
  session: Pick<school.StudySession, "studiedSeconds" | "timer" | "state">,
  nowMs: number = Date.now(),
): number {
  return timer.studiedSecondsAt(session, nowMs, offsetMs);
}

export { isBreak, type TimerView } from "@tokuly/core/school";
