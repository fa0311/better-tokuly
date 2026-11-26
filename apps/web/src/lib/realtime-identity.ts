import type { SessionState } from "@tokuly/api";

/**
 * Whom a session state lets the realtime connection speak for: a session's
 * ID, `""` for nobody (signed out), or `null` while that is not known yet.
 */
export function identityOf(state: SessionState): string | null {
  if (state.status === "signedIn") return state.sessionId;
  return state.status === "signedOut" ? "" : null;
}

/**
 * Tells `restart` when the session becomes someone else's: the connection
 * said hello as whoever was signed in when it opened, and keeps receiving
 * as them until it says hello again. Signing in on a page that was already
 * connected (a live stream watched signed out, then the sign-in form) would
 * otherwise bring no notifications or chat until the next reconnect.
 *
 * The first known state restarts nothing: a connection opened while the
 * session is still being restored waits for its token anyway.
 */
export function restartWhenSessionChanges(
  session: { state: SessionState; subscribe(listener: (state: SessionState) => void): () => void },
  restart: () => void,
): () => void {
  let identity = identityOf(session.state);
  return session.subscribe((state) => {
    const next = identityOf(state);
    if (next === null || next === identity) return;
    const known = identity !== null;
    identity = next;
    if (known) restart();
  });
}
