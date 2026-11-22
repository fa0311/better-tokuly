import { timestampFromMs } from "@bufbuild/protobuf/wkt";
import { create, type MessageInitShape, school } from "@tokuly/api";
import { beforeEach, describe, expect, it } from "vitest";
import {
  loadActiveSession,
  resetActiveSession,
  setActiveSession,
  setActiveTimer,
  useActiveSession,
} from "./session-store";

const T0 = Date.UTC(2026, 10, 24, 9, 0, 0);

function session(
  id: string,
  state: school.SessionState,
  serverMs?: number,
  extra: MessageInitShape<typeof school.StudySessionSchema> = {},
): school.StudySession {
  return create(school.StudySessionSchema, {
    id,
    state,
    timer:
      serverMs === undefined
        ? undefined
        : create(school.TimerSnapshotSchema, {
            mode: school.StudyMode.STOPWATCH,
            state: school.TimerState.RUNNING,
            serverTime: timestampFromMs(serverMs),
          }),
    ...extra,
  });
}

const { RUNNING, PAUSED, ENDED } = school.SessionState;
const stored = () => useActiveSession.getState().session;

describe("the active study session", () => {
  beforeEach(() => resetActiveSession());

  it("takes newer copies of a session and ignores an older one that arrives late", () => {
    expect(setActiveSession(session("a", RUNNING, T0 + 5_000))).toBe(true);
    // The answer to "pause" lands, then a push made before the pause.
    expect(setActiveSession(session("a", PAUSED, T0 + 9_000))).toBe(true);
    expect(setActiveSession(session("a", RUNNING, T0 + 6_000))).toBe(false);
    expect(stored()?.state).toBe(PAUSED);
    // The same moment again (the push of the very change) is fine.
    expect(setActiveSession(session("a", PAUSED, T0 + 9_000))).toBe(true);
  });

  it("clears the timer when its session ends, but not when an earlier session's end shows up late", () => {
    setActiveSession(session("b", RUNNING, T0 + 60_000));
    expect(setActiveSession(session("a", ENDED))).toBe(false);
    expect(stored()?.id).toBe("b");
    expect(setActiveSession(session("b", ENDED))).toBe(true);
    expect(stored()).toBeUndefined();
    expect(useActiveSession.getState().loaded).toBe(true);
  });

  it("follows a room's timer only for the room the session is in, and never backwards", () => {
    setActiveSession(session("a", RUNNING, T0 + 10_000, { roomId: "room" }));
    const timer = (serverMs: number) =>
      create(school.TimerSnapshotSchema, {
        state: school.TimerState.PAUSED,
        serverTime: timestampFromMs(serverMs),
      });

    setActiveTimer("other", timer(T0 + 20_000));
    expect(stored()?.timer?.state).toBe(school.TimerState.RUNNING);
    setActiveTimer("room", timer(T0 + 5_000));
    expect(stored()?.timer?.state).toBe(school.TimerState.RUNNING);
    setActiveTimer("room", timer(T0 + 20_000));
    expect(stored()?.timer?.state).toBe(school.TimerState.PAUSED);
  });

  it("restores the session from the server, and says so when the server cannot be asked", async () => {
    await loadActiveSession(() => Promise.reject(new Error("offline")));
    expect(useActiveSession.getState()).toMatchObject({ loaded: false, failed: true });

    await loadActiveSession(() =>
      Promise.resolve({ session: session("a", RUNNING, T0), heartbeatIntervalSeconds: 30 }),
    );
    expect(useActiveSession.getState()).toMatchObject({
      loaded: true,
      failed: false,
      heartbeatSeconds: 30,
    });
    expect(stored()?.id).toBe("a");

    // A later failure does not throw away what is known.
    await loadActiveSession(() => Promise.reject(new Error("offline")));
    expect(useActiveSession.getState()).toMatchObject({ loaded: true, failed: false });
    expect(stored()?.id).toBe("a");
  });

  it('drops an answer of "no session" that a start overtook', async () => {
    let answer: (value: { heartbeatIntervalSeconds: number }) => void = () => {};
    const loading = loadActiveSession(() => new Promise((resolve) => (answer = resolve)));
    // The timer is started while the question is still on its way.
    setActiveSession(session("a", RUNNING, T0));
    answer({ heartbeatIntervalSeconds: 60 });
    await loading;
    expect(stored()?.id).toBe("a");
  });
});
