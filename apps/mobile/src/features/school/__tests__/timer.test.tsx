import { timestampFromMs } from "@bufbuild/protobuf/wkt";
import { Code, ConnectError } from "@connectrpc/connect";
import { QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen, waitFor } from "@testing-library/react-native";
import { create, type MessageInitShape, school } from "@tokuly/api";
import { Alert, type AlertButton } from "react-native";
import { type Account, currentAccount, isActive } from "#/features/session/account";
import { api } from "#/lib/api";
import {
  askPushPermission,
  cancelReminder,
  readPushPermission,
  scheduleReminder,
} from "#/platform/notifications";
import { writeSecure } from "#/platform/secure-storage";
import { testQueryClient } from "#/testing/query";
import lifecycle from "../lifecycle";
import {
  followAccount,
  forgetReminder,
  heartbeat,
  remindBeforeIdle,
  stopAs,
  takeSession,
  useStudyTimer,
} from "../session";
import { TimerCard } from "../timer-card";

jest.mock("#/lib/api", () => ({ api: jest.fn(), apiWith: jest.fn(), authClient: jest.fn() }));
jest.mock("#/features/session/account", () => ({ currentAccount: jest.fn(), isActive: jest.fn() }));
jest.mock("#/platform/notifications", () => ({
  scheduleReminder: jest.fn(async () => {}),
  cancelReminder: jest.fn(async () => {}),
  readPushPermission: jest.fn(async () => "granted"),
  askPushPermission: jest.fn(async () => "granted"),
}));
jest.mock("expo-router", () => ({ router: { push: jest.fn() } }));

const T0 = Date.UTC(2026, 10, 27, 9, 0, 0);

function session(
  state: school.SessionState,
  serverMs = T0,
  extra: MessageInitShape<typeof school.StudySessionSchema> = {},
): school.StudySession {
  return create(school.StudySessionSchema, {
    id: "st1",
    state,
    source: school.SessionSource.TIMER,
    studiedSeconds: 60,
    timer: create(school.TimerSnapshotSchema, {
      mode: school.StudyMode.STOPWATCH,
      state:
        state === school.SessionState.PAUSED ? school.TimerState.PAUSED : school.TimerState.RUNNING,
      phase: school.TimerPhase.WORK,
      elapsedSeconds: 60,
      serverTime: timestampFromMs(serverMs),
    }),
    ...extra,
  });
}

const { RUNNING, PAUSED, ENDED } = school.SessionState;

/** One account's study service; what it was asked is listed in `calls`. */
function accountOf(sessionId: string, { heartbeatEnds = false } = {}) {
  const calls: string[] = [];
  const service = {
    getCurrentStudySession: async () => ({ heartbeatIntervalSeconds: 60, idleTimeoutSeconds: 900 }),
    getSchoolSettings: async () => ({ settings: undefined }),
    listSubjects: async () => ({ subjects: [] }),
    startStudySession: async (request: { mode: school.StudyMode }) => {
      calls.push(`start:${request.mode}`);
      return {
        session: session(RUNNING, Date.now()),
        heartbeatIntervalSeconds: 60,
        idleTimeoutSeconds: 900,
      };
    },
    pauseStudySession: async () => {
      calls.push("pause");
      return { session: session(PAUSED, Date.now()) };
    },
    stopStudySession: async () => {
      calls.push("stop");
      return { session: session(ENDED, Date.now(), { studiedSeconds: 1500 }) };
    },
    heartbeatStudySession: async () => {
      calls.push("heartbeat");
      return {
        session: heartbeatEnds
          ? session(ENDED, Date.now(), { autoStopped: true })
          : session(RUNNING, Date.now()),
        heartbeatIntervalSeconds: 60,
      };
    },
  };
  const client = { service: () => service };
  const account: Account = { sessionId, userId: `u-${sessionId}`, api: client as never };
  return { account, calls };
}

function signIn(account: Account) {
  jest.mocked(api).mockReturnValue(account.api);
  jest.mocked(currentAccount).mockReturnValue(account);
  jest.mocked(isActive).mockImplementation((other) => other.sessionId === account.sessionId);
  followAccount(account);
}

beforeEach(async () => {
  jest.spyOn(Alert, "alert").mockImplementation(() => {});
  jest.mocked(readPushPermission).mockResolvedValue("granted");
  await writeSecure("school.reminders.asked", null);
});

/** Answers the next alert with its button whose text is `text`. */
function answerAlert(text: string) {
  jest.mocked(Alert.alert).mockImplementationOnce((_title, _body, buttons?: AlertButton[]) => {
    buttons?.find((button) => button.text === text)?.onPress?.();
  });
}
afterEach(() => {
  jest.clearAllMocks();
  jest.restoreAllMocks();
});

describe("the timer's session", () => {
  it("never lets an older copy replace a newer one", () => {
    signIn(accountOf("s1").account);
    takeSession(session(RUNNING, T0 + 1000));
    expect(takeSession(session(PAUSED, T0))).toBe(false);
    expect(useStudyTimer.getState().session?.state).toBe(RUNNING);
  });

  it("starts over for another account", () => {
    signIn(accountOf("s1").account);
    takeSession(session(RUNNING));
    signIn(accountOf("s2").account);
    expect(useStudyTimer.getState()).toMatchObject({ session: undefined, loaded: false });
  });

  it("remembers that the server stopped a silent timer", async () => {
    const { account } = accountOf("s1", { heartbeatEnds: true });
    signIn(account);
    takeSession(session(RUNNING));
    await heartbeat(account);
    expect(useStudyTimer.getState()).toMatchObject({ session: undefined, autoStopped: true });
    expect(cancelReminder).toHaveBeenCalled();
  });

  it("drops a heartbeat's answer that comes back for an account that left", async () => {
    const first = accountOf("s1");
    signIn(first.account);
    takeSession(session(RUNNING));
    const beat = heartbeat(first.account);
    signIn(accountOf("s2").account);
    await beat;
    expect(useStudyTimer.getState().session).toBeUndefined();
  });

  it("reminds before the server's limit only while the timer runs", async () => {
    signIn(accountOf("s1").account);
    takeSession(session(PAUSED), { idleSeconds: 900 });
    await remindBeforeIdle();
    expect(scheduleReminder).not.toHaveBeenCalled();
    takeSession(session(RUNNING, T0 + 1));
    await remindBeforeIdle();
    expect(scheduleReminder).toHaveBeenCalledWith(
      "school.study.idle",
      expect.objectContaining({ path: "/school/study", channel: "school" }),
      780,
    );
  });

  it("takes back a reminder asked for a moment before the app came back", async () => {
    const done: string[] = [];
    jest.mocked(scheduleReminder).mockImplementationOnce(async () => {
      // The OS is asked for the permission first: the reminder lands a moment later.
      await new Promise((resolve) => setTimeout(resolve, 5));
      done.push("scheduled");
    });
    jest.mocked(cancelReminder).mockImplementation(async () => void done.push("taken back"));
    signIn(accountOf("s1").account);
    takeSession(session(RUNNING));
    done.length = 0;
    const leaving = remindBeforeIdle();
    await Promise.all([leaving, forgetReminder()]);
    expect(done).toEqual(["scheduled", "taken back"]);
  });
});

describe("leaving the device", () => {
  it("stops the timer with the leaving account's own API", async () => {
    const leaving = accountOf("s1");
    signIn(leaving.account);
    takeSession(session(RUNNING));
    await stopAs(leaving.account);
    expect(leaving.calls).toEqual(["stop"]);
    expect(useStudyTimer.getState().session).toBeUndefined();
    expect(cancelReminder).toHaveBeenCalled();
  });

  it("stops only a timer this phone kept running", async () => {
    const idle = accountOf("s1");
    signIn(idle.account);
    takeSession(undefined);
    await stopAs(idle.account);
    expect(idle.calls).toEqual([]);

    // Not loaded yet: this phone said nothing for it; another device may be running it.
    const unknown = accountOf("s2");
    signIn(unknown.account);
    await stopAs(unknown.account);
    expect(unknown.calls).toEqual([]);

    // Someone else's timer is on the screen: the leaving account's is not this phone's to stop.
    const parked = accountOf("s3");
    signIn(accountOf("s4").account);
    takeSession(session(RUNNING));
    await stopAs(parked.account);
    expect(parked.calls).toEqual([]);
    expect(useStudyTimer.getState().session).toBeDefined();
  });

  it("leaves a session that left long ago alone: its timer may be another device's by now", () => {
    expect(lifecycle).not.toHaveProperty("beforeEnd");
  });

  it("goes on when the server cannot be told", async () => {
    const leaving = accountOf("s1");
    const service = leaving.account.api.service(school.StudyService);
    jest
      .spyOn(service, "stopStudySession")
      .mockRejectedValue(new ConnectError("offline", Code.Unavailable));
    signIn(leaving.account);
    takeSession(session(RUNNING));
    await expect(stopAs(leaving.account)).resolves.toBeUndefined();
  });
});

describe("the timer card", () => {
  async function show() {
    await render(
      <QueryClientProvider
        client={testQueryClient({ defaultOptions: { queries: { retry: false } } })}
      >
        <TimerCard />
      </QueryClientProvider>,
    );
  }

  it("starts a pomodoro, pauses it and stops it after asking", async () => {
    const { account, calls } = accountOf("s1");
    signIn(account);
    takeSession(undefined);
    await show();
    await fireEvent.press(await screen.findByTestId("timer-mode-pomodoro"));
    await fireEvent.press(screen.getByTestId("timer-start"));
    expect(await screen.findByTestId("timer-clock")).toBeTruthy();
    expect(calls).toEqual([`start:${school.StudyMode.POMODORO}`]);

    await fireEvent.press(screen.getByTestId("timer-pause"));
    expect(await screen.findByTestId("timer-resume")).toBeTruthy();

    jest.mocked(Alert.alert).mockImplementationOnce((_title, _body, buttons?: AlertButton[]) => {
      buttons?.find((button) => button.style !== "cancel")?.onPress?.();
    });
    await fireEvent.press(screen.getByTestId("timer-stop"));
    expect(await screen.findByTestId("timer-start")).toBeTruthy();
    expect(calls).toEqual([`start:${school.StudyMode.POMODORO}`, "pause", "stop"]);
    await waitFor(() =>
      expect(Alert.alert).toHaveBeenLastCalledWith("25分 勉強したよ! おつかれさま!"),
    );
  });
});

describe("the reminder's permission", () => {
  async function show() {
    await render(
      <QueryClientProvider
        client={testQueryClient({ defaultOptions: { queries: { retry: false } } })}
      >
        <TimerCard />
      </QueryClientProvider>,
    );
  }

  it("is asked for, with the reason, the first time the timer starts, and only then", async () => {
    const { account, calls } = accountOf("s1");
    signIn(account);
    takeSession(undefined);
    jest.mocked(readPushPermission).mockResolvedValue("undetermined");
    answerAlert("お知らせをオンにする");
    await show();
    await fireEvent.press(await screen.findByTestId("timer-start"));
    expect(await screen.findByTestId("timer-clock")).toBeTruthy();
    expect(Alert.alert).toHaveBeenCalledWith(
      "止まる前にお知らせしてもいい?",
      expect.stringContaining("15分くらい"),
      expect.any(Array),
      { cancelable: false },
    );
    expect(askPushPermission).toHaveBeenCalledTimes(1);
    expect(calls).toEqual([`start:${school.StudyMode.STOPWATCH}`]);

    // The next start does not ask again, whatever was answered.
    takeSession(undefined);
    jest.mocked(Alert.alert).mockClear();
    await fireEvent.press(await screen.findByTestId("timer-start"));
    expect(await screen.findByTestId("timer-clock")).toBeTruthy();
    expect(Alert.alert).not.toHaveBeenCalled();
    expect(askPushPermission).toHaveBeenCalledTimes(1);
  });

  it("declined, the running timer says to keep the app open and offers to turn them on", async () => {
    const { account, calls } = accountOf("s1");
    signIn(account);
    takeSession(undefined);
    jest.mocked(readPushPermission).mockResolvedValue("undetermined");
    answerAlert("あとで");
    await show();
    await fireEvent.press(await screen.findByTestId("timer-start"));
    expect(await screen.findByTestId("timer-reminder-off")).toBeTruthy();
    expect(askPushPermission).not.toHaveBeenCalled();
    expect(calls).toEqual([`start:${school.StudyMode.STOPWATCH}`]);

    await fireEvent.press(screen.getByTestId("timer-reminder-on"));
    await waitFor(() => expect(screen.queryByTestId("timer-reminder-off")).toBeNull());
    expect(askPushPermission).toHaveBeenCalledTimes(1);
  });

  it("allowed, the running timer shows no warning", async () => {
    signIn(accountOf("s1").account);
    takeSession(session(RUNNING, Date.now()));
    await show();
    expect(await screen.findByTestId("timer-clock")).toBeTruthy();
    await waitFor(() => expect(readPushPermission).toHaveBeenCalled());
    expect(screen.queryByTestId("timer-reminder-off")).toBeNull();
  });
});
