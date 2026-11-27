import { timestampFromMs } from "@bufbuild/protobuf/wkt";
import { QueryClientProvider } from "@tanstack/react-query";
import { act, render } from "@testing-library/react-native";
import { create, school } from "@tokuly/api";
import { AppState, type AppStateStatus } from "react-native";
import { type Account, currentAccount, isActive } from "#/features/session/account";
import { cancelReminder, scheduleReminder } from "#/platform/notifications";
import { testQueryClient } from "#/testing/query";
import SchoolBackground from "../background";
import { reminderFor, takeSession } from "../session";

jest.mock("#/features/session/account", () => ({ currentAccount: jest.fn(), isActive: jest.fn() }));
jest.mock("#/lib/realtime", () => ({
  realtime: () => ({
    onFrame: () => () => {},
    start: () => {},
    subscribe: () => ({ release: () => {} }),
  }),
}));
jest.mock("#/platform/notifications", () => ({
  scheduleReminder: jest.fn(async () => {}),
  cancelReminder: jest.fn(async () => {}),
}));

const { RUNNING, PAUSED } = school.SessionState;

function session(state: school.SessionState): school.StudySession {
  return create(school.StudySessionSchema, {
    id: "st1",
    state,
    timer: create(school.TimerSnapshotSchema, {
      mode: school.StudyMode.STOPWATCH,
      state: state === PAUSED ? school.TimerState.PAUSED : school.TimerState.RUNNING,
      serverTime: timestampFromMs(Date.now()),
    }),
  });
}

/** A pomodoro (25/5, four rounds) `elapsedMinutes` in, as the server sends it now. */
function pomodoro(elapsedMinutes: number): school.StudySession {
  const inBreak = elapsedMinutes % 30 >= 25;
  const endsInMs = ((inBreak ? 30 : 25) - (elapsedMinutes % 30)) * 60_000;
  return create(school.StudySessionSchema, {
    id: "st1",
    state: RUNNING,
    timer: create(school.TimerSnapshotSchema, {
      mode: school.StudyMode.POMODORO,
      state: school.TimerState.RUNNING,
      phase: inBreak ? school.TimerPhase.BREAK : school.TimerPhase.WORK,
      elapsedSeconds: elapsedMinutes * 60,
      pomodoro: create(school.PomodoroConfigSchema, {
        workMinutes: 25,
        breakMinutes: 5,
        longBreakMinutes: 15,
        rounds: 4,
      }),
      serverTime: timestampFromMs(Date.now()),
      phaseEndsAt: timestampFromMs(Date.now() + endsInMs),
    }),
  });
}

/**
 * The phone with someone signed in whose timer the server has as `kept`
 * (a heartbeat answers with `beat`); `calls` lists what it was asked.
 */
async function open(
  kept: school.StudySession | undefined,
  beat = () => kept && session(kept.state),
) {
  const calls: string[] = [];
  const service = {
    getCurrentStudySession: async () => {
      calls.push("current");
      return { session: kept, heartbeatIntervalSeconds: 60, idleTimeoutSeconds: 900 };
    },
    heartbeatStudySession: async () => {
      calls.push("heartbeat");
      return { session: beat(), heartbeatIntervalSeconds: 60 };
    },
  };
  const account: Account = {
    sessionId: "s1",
    userId: "u1",
    api: { service: () => service } as never,
  };
  jest.mocked(currentAccount).mockReturnValue(account);
  jest.mocked(isActive).mockReturnValue(true);
  let appState: (state: AppStateStatus) => void = () => {};
  jest.spyOn(AppState, "addEventListener").mockImplementation((_type, listener) => {
    appState = listener as typeof appState;
    return { remove: () => {} } as never;
  });
  AppState.currentState = "active";
  const view = await render(
    <QueryClientProvider client={testQueryClient()}>
      <SchoolBackground />
    </QueryClientProvider>,
  );
  await act(async () => {});
  return {
    calls,
    view,
    to: (state: AppStateStatus) => act(async () => appState(state)),
    /** Lets `ms` of the phone's timers pass. */
    wait: (ms: number) => act(async () => void (await jest.advanceTimersByTimeAsync(ms))),
  };
}

beforeEach(() => {
  jest.useFakeTimers();
});
afterEach(async () => {
  takeSession(undefined);
  jest.useRealTimers();
  jest.clearAllMocks();
  jest.restoreAllMocks();
});

describe("the study timer in the background of the app", () => {
  it("speaks for a timer found running the moment it is found, then every interval", async () => {
    // Opened from the reminder minutes before the server's limit: the next interval would be too late.
    const phone = await open(session(RUNNING));
    expect(phone.calls).toEqual(["current", "heartbeat"]);
    await phone.wait(60_000);
    expect(phone.calls).toEqual(["current", "heartbeat", "heartbeat"]);
    await phone.view.unmount();
  });

  it("off screen: a last heartbeat and the reminder; back: the reminder goes and a heartbeat counts the gap", async () => {
    const phone = await open(session(RUNNING));
    phone.calls.length = 0;
    await phone.to("background");
    expect(phone.calls).toEqual(["heartbeat"]);
    expect(scheduleReminder).toHaveBeenCalledTimes(1);
    expect(scheduleReminder).toHaveBeenCalledWith("school.study.idle", expect.anything(), 780);

    // Nothing is sent from the background.
    await phone.wait(5 * 60_000);
    expect(phone.calls).toEqual(["heartbeat"]);

    jest.mocked(cancelReminder).mockClear();
    await phone.to("active");
    expect(cancelReminder).toHaveBeenCalledWith("school.study.idle");
    expect(phone.calls).toEqual(["heartbeat", "heartbeat"]);
    await phone.view.unmount();
  });

  it("off screen in a pomodoro: the reminder comes when the phase ends, not before the stopwatch's limit", async () => {
    const phone = await open(pomodoro(10), () => pomodoro(10));
    await phone.to("background");
    expect(scheduleReminder).toHaveBeenCalledWith(
      "school.study.idle",
      expect.objectContaining({ title: "集中タイムおわり! おつかれさま" }),
      15 * 60,
    );
    await phone.view.unmount();
  });

  it("sends nothing and reminds of nothing without a timer", async () => {
    const phone = await open(undefined);
    await phone.to("background");
    await phone.wait(120_000);
    expect(phone.calls).toEqual(["current"]);
    expect(scheduleReminder).not.toHaveBeenCalled();
    await phone.view.unmount();
  });

  it("stops sending when the signed-in tree goes", async () => {
    const phone = await open(session(RUNNING));
    await phone.view.unmount();
    phone.calls.length = 0;
    await phone.wait(180_000);
    expect(phone.calls).toEqual([]);
  });
});

describe("when the reminder comes", () => {
  it("in a break: at its end", () => {
    const kept = pomodoro(27);
    takeSession(kept);
    expect(reminderFor(kept, 900, Date.now())).toEqual({
      seconds: 180,
      key: "mobile:school.phaseReminder.break",
    });
  });

  it("never later than shortly before the server's limit, counted from the phase last heard", () => {
    // Last heard at minute 24 of a 4×25 pomodoro; going off screen at the
    // fourth round's end (the long break of 15 minutes is longer than what
    // is left of the limit, counted from minute 25).
    const kept = pomodoro(24 + 90);
    takeSession(kept);
    jest.setSystemTime(Date.now() + 60_000);
    expect(reminderFor(kept, 900, Date.now())).toEqual({
      seconds: 13 * 60,
      key: "mobile:school.idleReminder",
    });
  });

  it("a stopwatch: shortly before the limit", () => {
    const kept = session(RUNNING);
    takeSession(kept);
    expect(reminderFor(kept, 900, Date.now())).toEqual({
      seconds: 780,
      key: "mobile:school.idleReminder",
    });
  });
});
