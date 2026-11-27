import { errorReason, school, toMillis } from "@tokuly/api";
import { replacesSession } from "@tokuly/core/school";
import { create } from "zustand";
import { type Account, isActive } from "#/features/session/account";
import i18n from "#/lib/i18n";
import { cancelReminder, scheduleReminder } from "#/platform/notifications";
import { syncClock } from "./clock";

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
  /** Silence after which the server stops the session. */
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

/**
 * Takes a session the server returned or pushed: an ended one clears the
 * timer, `undefined` means the server has none. An older copy never
 * replaces a newer one (`replacesSession`). Returns whether it was taken.
 */
export function takeSession(
  session: school.StudySession | undefined,
  limits: { heartbeatSeconds?: number; idleSeconds?: number } = {},
): boolean {
  const state = useStudyTimer.getState();
  if (!replacesSession(session, state.session)) return false;
  syncClock(session?.timer);
  const ended = session?.state === school.SessionState.ENDED;
  useStudyTimer.setState({
    session: session && !ended ? session : undefined,
    loaded: true,
    failed: false,
    heartbeatSeconds: limits.heartbeatSeconds || state.heartbeatSeconds,
    idleSeconds: limits.idleSeconds || state.idleSeconds,
    autoStopped: state.autoStopped || (ended && session.autoStopped),
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
/** How long before the server's limit the reminder comes: time to pick the phone up. */
const REMINDER_LEAD_SECONDS = 120;

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
    await scheduleReminder(
      REMINDER_ID,
      {
        title: i18n.t("mobile:school.idleReminder.title"),
        body: i18n.t("mobile:school.idleReminder.body"),
        path: "/school/study",
        channel: "school",
      },
      Math.max(60, idleSeconds - REMINDER_LEAD_SECONDS),
    );
  });
}

export function forgetReminder(): Promise<void> {
  return inTurn(() => cancelReminder(REMINDER_ID));
}

/**
 * The account leaves the device: its timer stops and is logged with its own
 * API (a timer the server cannot hear from would only be stopped by it
 * later, and friends would see it studying until then).
 */
export async function stopAs(account: Account): Promise<void> {
  const mine = owner?.sessionId === account.sessionId;
  const { session, loaded } = useStudyTimer.getState();
  // Only a timer known to be off is left alone: one not loaded yet may be running.
  const idle = mine && loaded && !session;
  if (mine) {
    owner = null;
    useStudyTimer.setState(INITIAL);
  }
  await forgetReminder();
  if (!idle) await stopTimer(account.api);
}

/** Stops whatever timer `api`'s account has; an account without one, or offline, is fine as it is. */
export async function stopTimer(api: Account["api"]): Promise<void> {
  try {
    await api.service(school.StudyService).stopStudySession({});
  } catch {
    // None running, or offline: the server stops a silent timer by itself, counted until the last heartbeat.
  }
}
