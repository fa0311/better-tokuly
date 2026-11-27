import { errorReason, school, toMillis } from "@tokuly/api";
import { isBreak, replacesSession } from "@tokuly/core/school";
import { create } from "zustand";
import { type Account, isActive } from "#/features/session/account";
import i18n from "#/lib/i18n";
import { cancelReminder, scheduleReminder } from "#/platform/notifications";
import { phaseEndAt, syncClock, timerAt } from "./clock";

/**
 * The signed-in account's study timer on this phone. The server keeps the
 * time; this keeps its last copy of the session, tells the server "still
 * here" while the app is open, and reminds the person before the server
 * gives up on a timer the app could not speak for (the phone does not let
 * an app send in the background).
 *
 * It belongs to one account (`owner`): every request goes through that
 * account's API, and it ends before the account leaves the device
 * (`lifecycle.ts`).
 */
interface StudyTimer {
  /** The running or paused session; `undefined` when not studying. */
  session: school.StudySession | undefined;
  /** False until the server said whether there is one. */
  loaded: boolean;
  /** The server could not be asked; the timer offers to try again. */
  failed: boolean;
  /** How often the server wants to hear "still here". */
  heartbeatSeconds: number;
  /** Silence after which the server stops the session (for a pomodoro, after the phase it was last heard in). */
  idleSeconds: number;
  /** The server stopped the timer by itself; until the person has seen it. */
  autoStopped: boolean;
  /** Counts every accepted change, so an answer asked for before one can be told apart. */
  revision: number;
}

const INITIAL: StudyTimer = {
  session: undefined,
  loaded: false,
  failed: false,
  heartbeatSeconds: 60,
  idleSeconds: 900,
  autoStopped: false,
  revision: 0,
};

export const useStudyTimer = create<StudyTimer>(() => INITIAL);

let owner: Account | null = null;

const studyOf = (account: Account) => account.api.service(school.StudyService);

/** The timer now follows `account`: what another account left in it is gone. */
export function followAccount(account: Account): void {
  if (owner?.sessionId === account.sessionId && owner.api === account.api) return;
  owner = account;
  useStudyTimer.setState(INITIAL);
}

/** The account whose timer this is, while it is still the one in use. */
export function timerAccount(): Account | null {
  return owner && isActive(owner) ? owner : null;
}

/** When the server last showed the timer alive, on this phone's clock. */
let heardAtMs = 0;

/**
 * Takes a session the server returned or pushed: an ended one clears the
 * timer, `undefined` means the server has none. An older copy never
 * replaces a newer one (`replacesSession`). Returns whether it was taken.
 *
 * A running timer the server no longer has after a silence longer than its
 * limit was stopped by the server (the phone was off screen when it said
 * so): the person is told, as when the heartbeat itself brings the news.
 */
export function takeSession(
  session: school.StudySession | undefined,
  limits: { heartbeatSeconds?: number; idleSeconds?: number } = {},
): boolean {
  const state = useStudyTimer.getState();
  if (!replacesSession(session, state.session)) return false;
  syncClock(session?.timer);
  const ended = session?.state === school.SessionState.ENDED;
  const outlived =
    !session &&
    state.session?.state === school.SessionState.RUNNING &&
    Date.now() - heardAtMs >= state.idleSeconds * 1000;
  if (session && !ended) heardAtMs = Date.now();
  useStudyTimer.setState({
    session: session && !ended ? session : undefined,
    loaded: true,
    failed: false,
    heartbeatSeconds: limits.heartbeatSeconds || state.heartbeatSeconds,
    idleSeconds: limits.idleSeconds || state.idleSeconds,
    autoStopped: state.autoStopped || outlived || (ended && session.autoStopped),
    revision: state.revision + 1,
  });
  if (!session || ended) void forgetReminder();
  return true;
}

/** A room's timer changed: the session spent in it follows (unless it already has a newer one). */
export function takeRoomTimer(roomId: string, timer: school.TimerSnapshot): void {
  const state = useStudyTimer.getState();
  const kept = state.session;
  if (!kept || kept.roomId !== roomId) return;
  if (toMillis(timer.serverTime) < toMillis(kept.timer?.serverTime)) return;
  syncClock(timer);
  useStudyTimer.setState({ session: { ...kept, timer }, revision: state.revision + 1 });
}

export function seenAutoStop(): void {
  useStudyTimer.setState({ autoStopped: false });
}

/**
 * Asks the server for the session (on start, when the connection comes
 * back, on "try again"). An answer of "none" overtaken by a change made in
 * the meantime is dropped; a failure is remembered only while nothing is known.
 */
export async function loadSession(account: Account): Promise<void> {
  const before = useStudyTimer.getState().revision;
  try {
    const response = await studyOf(account).getCurrentStudySession({});
    if (!isActive(account) || owner !== account) return;
    if (!response.session && useStudyTimer.getState().revision !== before) return;
    takeSession(response.session, {
      heartbeatSeconds: response.heartbeatIntervalSeconds,
      idleSeconds: response.idleTimeoutSeconds,
    });
  } catch {
    if (isActive(account) && !useStudyTimer.getState().loaded)
      useStudyTimer.setState({ failed: true });
  }
}

/** "Still here": the server counts study time up to the last heartbeat only. */
export async function heartbeat(account: Account): Promise<void> {
  try {
    const response = await studyOf(account).heartbeatStudySession({});
    if (!isActive(account) || owner !== account) return;
    takeSession(response.session, { heartbeatSeconds: response.heartbeatIntervalSeconds });
  } catch (error) {
    if (isActive(account) && owner === account && errorReason(error) === "NO_ACTIVE_SESSION")
      takeSession(undefined);
  }
}

const REMINDER_ID = "school.study.idle";
/** How long before the server's limit the reminder comes at the latest: time to pick the phone up. */
const REMINDER_LEAD_SECONDS = 120;

/**
 * When the reminder for a timer going off screen now comes, in seconds from
 * now, and which one it is. The server gives up the idle limit after the
 * last sign of life — for a running pomodoro, after the end of the phase it
 * was last heard in. A pomodoro is reminded at the end of its phase, a
 * stopwatch shortly before the limit; never later than that.
 */
export function reminderFor(
  session: school.StudySession,
  idleSeconds: number,
  nowMs: number,
): { seconds: number; key: string } {
  const believedMs = Math.max(heardAtMs, phaseEndAt(session.timer, heardAtMs) ?? 0);
  const latestMs = believedMs + (idleSeconds - REMINDER_LEAD_SECONDS) * 1000;
  const phaseEndMs = phaseEndAt(session.timer, nowMs);
  const key =
    phaseEndMs !== undefined && phaseEndMs <= latestMs
      ? isBreak(timerAt(session.timer, nowMs).phase)
        ? "mobile:school.phaseReminder.break"
        : "mobile:school.phaseReminder.work"
      : "mobile:school.idleReminder";
  const atMs = Math.min(phaseEndMs ?? latestMs, latestMs);
  return { seconds: Math.max(60, (atMs - nowMs) / 1000), key };
}

/**
 * Asking for the reminder and taking it back happen in the order they were
 * asked for: scheduling first reads the permission, so without the queue a
 * reminder asked for on the way out could land after the return took it back.
 */
let reminderQueue: Promise<void> = Promise.resolve();

function inTurn(work: () => Promise<void>): Promise<void> {
  // A failure is fine: without the reminder the server still keeps the time counted until the last heartbeat.
  reminderQueue = reminderQueue.then(work).catch(() => {});
  return reminderQueue;
}

/**
 * The app goes off screen with the timer running: it cannot speak for the
 * timer from there, so the person hears about it before the server stops it.
 */
export function remindBeforeIdle(): Promise<void> {
  return inTurn(async () => {
    const { session, idleSeconds } = useStudyTimer.getState();
    // A paused timer waits for its own, longer limit; it needs no heartbeat.
    if (!session || session.state !== school.SessionState.RUNNING) return;
    const reminder = reminderFor(session, idleSeconds, Date.now());
    await scheduleReminder(
      REMINDER_ID,
      {
        title: i18n.t(`${reminder.key}.title`),
        body: i18n.t(`${reminder.key}.body`),
        path: "/school/study",
        channel: "school",
      },
      reminder.seconds,
    );
  });
}

export function forgetReminder(): Promise<void> {
  return inTurn(() => cancelReminder(REMINDER_ID));
}

/**
 * The account leaves the device: the timer this phone kept running for it
 * stops and is logged with its own API (the server would only stop a timer
 * it cannot hear from later, and friends would see it studying until then).
 *
 * Only that one: a timer this phone does not hold (not loaded yet, or the
 * store follows someone else) it said nothing for, and the account may be
 * running one on another device.
 */
export async function stopAs(account: Account): Promise<void> {
  if (owner?.sessionId !== account.sessionId) return;
  const { session } = useStudyTimer.getState();
  owner = null;
  useStudyTimer.setState(INITIAL);
  await forgetReminder();
  if (!session) return;
  try {
    await account.api.service(school.StudyService).stopStudySession({});
  } catch {
    // None running anymore, or offline: the server stops a silent timer by itself, counted until the last heartbeat.
  }
}
