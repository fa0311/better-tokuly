import { school } from "@tokuly/api";
import { create } from "zustand";
import { syncClock } from "./timer";

interface ActiveSession {
  /** The caller's running or paused session; `undefined` when they are not studying. */
  session: school.StudySession | undefined;
  /** False until the server said whether there is one (after a reload). */
  loaded: boolean;
  /** How often the server wants to hear "still here". */
  heartbeatSeconds: number;
}

/**
 * The signed-in person's active study session. It lives outside the pages so
 * the timer keeps its heartbeat whichever page is open; `background.tsx`
 * keeps it current from the API and the realtime connection.
 */
export const useActiveSession = create<ActiveSession>(() => ({ session: undefined, loaded: false, heartbeatSeconds: 60 }));

/** Takes a session the server returned or pushed: an ended one clears the timer. */
export function setActiveSession(session: school.StudySession | undefined, heartbeatSeconds?: number): void {
  syncClock(session?.timer);
  const active = session && session.state !== school.SessionState.ENDED ? session : undefined;
  useActiveSession.setState((state) => ({ session: active, loaded: true, heartbeatSeconds: heartbeatSeconds || state.heartbeatSeconds }));
}

/** A room's timer changed: the session of someone inside follows it. */
export function setActiveTimer(roomId: string, timer: school.TimerSnapshot): void {
  syncClock(timer);
  useActiveSession.setState((state) => (state.session && state.session.roomId === roomId ? { session: { ...state.session, timer } } : state));
}

export function resetActiveSession(): void {
  useActiveSession.setState({ session: undefined, loaded: false });
}
