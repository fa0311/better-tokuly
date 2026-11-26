import type { SessionState } from "@tokuly/api";
import { describe, expect, it, vi } from "vitest";
import { restartWhenSessionChanges } from "./realtime-identity";

const signedIn = (sessionId: string) => ({ status: "signedIn", sessionId, me: {} }) as SessionState;

function session(initial: SessionState) {
  const listeners = new Set<(state: SessionState) => void>();
  return {
    state: initial,
    subscribe(listener: (state: SessionState) => void) {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    set(state: SessionState) {
      this.state = state;
      for (const listener of listeners) listener(state);
    },
  };
}

describe("the realtime connection follows the session", () => {
  it("says hello again when someone signs in on a connected page", () => {
    const current = session({ status: "signedOut" });
    const restart = vi.fn();
    restartWhenSessionChanges(current, restart);
    current.set(signedIn("s1"));
    expect(restart).toHaveBeenCalledTimes(1);
  });

  it("says hello again after signing out, and as the next session", () => {
    const current = session(signedIn("s1"));
    const restart = vi.fn();
    restartWhenSessionChanges(current, restart);
    current.set({ status: "signedOut" });
    current.set(signedIn("s2"));
    expect(restart).toHaveBeenCalledTimes(2);
  });

  it("leaves the connection alone while nothing changes", () => {
    const current = session({ status: "unknown" });
    const restart = vi.fn();
    restartWhenSessionChanges(current, restart);
    // Restoring the session at start: the connection waits for its token anyway.
    current.set(signedIn("s1"));
    // The profile changed, the token was refreshed: the same session.
    current.set(signedIn("s1"));
    current.set({ status: "unknown" });
    current.set(signedIn("s1"));
    expect(restart).not.toHaveBeenCalled();
  });
});
