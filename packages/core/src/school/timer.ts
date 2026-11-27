import { school, toMillis } from "@tokuly/api";

/**
 * The client side of the server's timers.
 *
 * The server keeps the time: a `TimerSnapshot` says where a timer stood at
 * `server_time`. A client only needs to know how far its own clock is from
 * the server's (`clockOffset`, kept by the app) and can then draw the timer
 * at any later moment — after a minute in the background, after a reload —
 * without counting ticks.
 */

/**
 * How far the server's clock is ahead of this client's, from a snapshot that
 * just arrived; `undefined` when the snapshot carries no time.
 */
export function clockOffset(
  snapshot: school.TimerSnapshot | undefined,
  receivedAtMs: number,
): number | undefined {
  const server = toMillis(snapshot?.serverTime);
  return server > 0 ? server - receivedAtMs : undefined;
}

interface Cycle {
  workMs: number;
  breakMs: number;
  longBreakMs: number;
  rounds: number;
}

function cycleOf(config: school.PomodoroConfig | undefined): Cycle {
  return {
    workMs: (config?.workMinutes || 25) * 60_000,
    breakMs: (config?.breakMinutes || 5) * 60_000,
    longBreakMs: (config?.longBreakMinutes || 15) * 60_000,
    rounds: config?.rounds || 4,
  };
}

interface PhaseAt {
  phase: school.TimerPhase;
  /** 1-based number of the work phase in progress, or coming up after a break. */
  round: number;
  startMs: number;
  endMs: number;
}

/** The phase a pomodoro timeline is in at a position (the server's rule, mirrored). */
function phaseAt(cycle: Cycle, positionMs: number): PhaseAt {
  const position = Math.max(0, positionMs);
  const length =
    cycle.rounds * cycle.workMs + (cycle.rounds - 1) * cycle.breakMs + cycle.longBreakMs;
  const cycleIndex = Math.floor(position / length);
  const cycleStart = cycleIndex * length;
  const step = cycle.workMs + cycle.breakMs;
  // The last round of a cycle is followed by the long break instead of a short one.
  const round = Math.min(Math.floor((position - cycleStart) / step), cycle.rounds - 1);
  const workStart = cycleStart + round * step;
  const workEnd = workStart + cycle.workMs;
  const firstRound = cycleIndex * cycle.rounds + 1;
  if (position < workEnd)
    return {
      phase: school.TimerPhase.WORK,
      round: firstRound + round,
      startMs: workStart,
      endMs: workEnd,
    };
  const long = round === cycle.rounds - 1;
  return {
    phase: long ? school.TimerPhase.LONG_BREAK : school.TimerPhase.BREAK,
    round: firstRound + round + 1,
    startMs: workEnd,
    endMs: workEnd + (long ? cycle.longBreakMs : cycle.breakMs),
  };
}

/** Milliseconds of work phases between two positions of a pomodoro timeline. */
function workBetween(cycle: Cycle, fromMs: number, toMs: number): number {
  let total = 0;
  let cursor = Math.max(0, fromMs);
  while (cursor < toMs) {
    const phase = phaseAt(cycle, cursor);
    const until = Math.min(phase.endMs, toMs);
    if (phase.phase === school.TimerPhase.WORK) total += until - cursor;
    cursor = until;
  }
  return total;
}

/** A timer as it stands at one moment. */
export interface TimerView {
  mode: school.StudyMode;
  state: school.TimerState;
  phase: school.TimerPhase;
  round: number;
  /** Rounds before the long break (pomodoro). */
  rounds: number;
  /** Time on the timer's timeline: running time, pauses excluded, breaks included. */
  elapsedMs: number;
  /** Pomodoro: time left in the phase, and the phase's whole length. */
  remainingMs: number;
  phaseMs: number;
  /** Study time counted since the snapshot was taken (work phases only). */
  workSinceMs: number;
}

/**
 * Where a timer stands at `nowMs` (the client's clock, `offsetMs` behind the
 * server's), given the last snapshot the server sent. A running pomodoro
 * moves through its phases by itself, exactly as the server computes them.
 */
export function timerAt(
  snapshot: school.TimerSnapshot | undefined,
  nowMs: number,
  offsetMs = 0,
): TimerView {
  const mode = snapshot?.mode || school.StudyMode.STOPWATCH;
  const state = snapshot?.state || school.TimerState.IDLE;
  const running = state === school.TimerState.RUNNING;
  const since = running ? Math.max(0, nowMs + offsetMs - toMillis(snapshot?.serverTime)) : 0;

  if (mode !== school.StudyMode.POMODORO) {
    const elapsedMs = (snapshot?.elapsedSeconds ?? 0) * 1000 + since;
    return {
      mode,
      state,
      phase: school.TimerPhase.WORK,
      round: 1,
      rounds: 1,
      elapsedMs,
      remainingMs: 0,
      phaseMs: 0,
      workSinceMs: since,
    };
  }

  const cycle = cycleOf(snapshot?.pomodoro);
  // `elapsed_seconds` is whole seconds; the phase's end pins the position to the millisecond.
  let base = (snapshot?.elapsedSeconds ?? 0) * 1000;
  const reported = phaseAt(cycle, base);
  const endsAt = toMillis(snapshot?.phaseEndsAt);
  if (running && endsAt > 0 && reported.phase === snapshot?.phase) {
    const pinned = reported.endMs - (endsAt - toMillis(snapshot?.serverTime));
    if (pinned >= reported.startMs && pinned < reported.endMs) base = pinned;
  }
  const position = base + since;
  const current = phaseAt(cycle, position);
  return {
    mode,
    state,
    phase: current.phase,
    round: current.round,
    rounds: cycle.rounds,
    elapsedMs: position,
    remainingMs: current.endMs - position,
    phaseMs: current.endMs - current.startMs,
    workSinceMs: workBetween(cycle, base, position),
  };
}

/** A session's counted study time at `nowMs`, in seconds. */
export function studiedSecondsAt(
  session: Pick<school.StudySession, "studiedSeconds" | "timer" | "state">,
  nowMs: number,
  offsetMs = 0,
): number {
  if (session.state !== school.SessionState.RUNNING) return session.studiedSeconds;
  return (
    session.studiedSeconds + Math.floor(timerAt(session.timer, nowMs, offsetMs).workSinceMs / 1000)
  );
}

export function isBreak(phase: school.TimerPhase): boolean {
  return phase === school.TimerPhase.BREAK || phase === school.TimerPhase.LONG_BREAK;
}

/** The words for a timer's phase (`timer.phase.<key>`): 集中, 休憩, 長い休憩, 一時停止中, まだ. */
export function phaseKey(view: Pick<TimerView, "phase" | "state">): string {
  if (view.state === school.TimerState.IDLE) return "idle";
  if (view.state === school.TimerState.PAUSED) return "paused";
  if (view.phase === school.TimerPhase.LONG_BREAK) return "longBreak";
  return view.phase === school.TimerPhase.BREAK ? "break" : "work";
}

const serverTime = (session: school.StudySession | undefined) =>
  toMillis(session?.timer?.serverTime);

/**
 * Whether a copy of the caller's session that just arrived replaces the one
 * kept. Copies arrive by several roads (an answer to a tap, a heartbeat, a
 * realtime push) and not always in the order they were made. Each carries
 * the server's clock, so an older copy never replaces a newer one, and the
 * end of an earlier session never clears the one that followed it.
 */
export function replacesSession(
  incoming: school.StudySession | undefined,
  kept: school.StudySession | undefined,
): boolean {
  if (!incoming || !kept) return true;
  const ended = incoming.state === school.SessionState.ENDED;
  if (ended) return incoming.id === kept.id;
  const at = serverTime(incoming);
  return !(at > 0 && at < serverTime(kept));
}
