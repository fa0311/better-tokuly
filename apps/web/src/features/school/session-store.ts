import { school, toMillis } from "@tokuly/api";
import { replacesSession } from "@tokuly/core/school";
import { create } from "zustand";
import { syncClock } from "./timer";

interface ActiveSession {
  /** The caller's running or paused session; `undefined` when they are not studying. */
  session: school.StudySession | undefined;
  /** False until the server said whether there is one (after a reload). */
  loaded: boolean;
  /** The server could not be asked whether there is one; the timer offers to try again. */
  failed: boolean;
  /** How often the server wants to hear "still here". */
  heartbeatSeconds: number;
  /** Counts every accepted change, so an answer that was asked for before one can be told apart. */
  revision: number;
}

/**
 * The signed-in person's active study session. It lives outside the pages so
 * the timer keeps its heartbeat whichever page is open; `background.tsx`
 * keeps it current from the API and the realtime connection.
 */
export const useActiveSession = create<ActiveSession>(() => ({
  session: undefined,
  loaded: false,
  failed: false,
  heartbeatSeconds: 60,
  revision: 0,
}));

const serverTime = (session: school.StudySession | undefined) =>
  toMillis(session?.timer?.serverTime);

/**
 * Takes a session the server returned or pushed: an ended one clears the
 * timer, `undefined` means the server has none.
 *
 * An older copy never replaces a newer one, and the end of an earlier
 * session never clears the one that followed it (`replacesSession`).
 * Returns whether the session was taken.
 */
export function setActiveSession(
  session: school.StudySession | undefined,
  heartbeatSeconds?: number,
): boolean {
  const state = useActiveSession.getState();
  if (!replacesSession(session, state.session)) return false;
  syncClock(session?.timer);
  const active = session && session.state !== school.SessionState.ENDED ? session : undefined;
  useActiveSession.setState({
    session: active,
    loaded: true,
    failed: false,
    heartbeatSeconds: heartbeatSeconds || state.heartbeatSeconds,
    revision: state.revision + 1,
  });
  return true;
}

/** A room's timer changed: the session of someone inside follows it (unless it already has a newer one). */
export function setActiveTimer(roomId: string, timer: school.TimerSnapshot): void {
  const state = useActiveSession.getState();
  if (
    !state.session ||
    state.session.roomId !== roomId ||
    toMillis(timer.serverTime) < serverTime(state.session)
  )
    return;
  syncClock(timer);
  useActiveSession.setState({ session: { ...state.session, timer }, revision: state.revision + 1 });
}

/** Counts the questions `loadActiveSession` asked: only the latest one's answer counts. */
let loads = 0;

/**
 * Asks the server for the caller's session (after a reload, on every
 * Welcome of the connection, on "try again"). A question asked again
 * replaces the one before: an older answer, even one with a session, may
 * predate a stop the newer one knows of. An answer of "none" that was
 * overtaken by a change made in the meantime is dropped too; a failure is
 * remembered so the timer can say so instead of waiting forever.
 */
export async function loadActiveSession(
  fetchCurrent: () => Promise<{ session?: school.StudySession; heartbeatIntervalSeconds: number }>,
): Promise<void> {
  const load = ++loads;
  const before = useActiveSession.getState().revision;
  try {
    const response = await fetchCurrent();
    if (load !== loads) return;
    if (!response.session && useActiveSession.getState().revision !== before) return;
    setActiveSession(response.session, response.heartbeatIntervalSeconds);
  } catch {
    // Something known beats a failed question; only a timer that knows nothing says it could not load.
    if (load === loads && !useActiveSession.getState().loaded)
      useActiveSession.setState({ failed: true });
  }
}

export function resetActiveSession(): void {
  useActiveSession.setState({ session: undefined, loaded: false, failed: false, revision: 0 });
}
