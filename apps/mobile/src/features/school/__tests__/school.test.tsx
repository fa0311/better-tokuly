import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen, waitFor } from "@testing-library/react-native";
import { create, fromDate, school } from "@tokuly/api";
import { router } from "expo-router";
import type { ReactNode } from "react";
import { Alert } from "react-native";
import { SafeAreaProvider } from "react-native-safe-area-context";
import { currentAccount, isActive } from "#/features/session/account";
import { api } from "#/lib/api";
import { AssignmentsScreen } from "../assignments-screen";
import { NewAssignmentScreen } from "../new-assignment-screen";
import { SchoolScreen } from "../today-screen";

jest.mock("#/lib/api", () => ({ api: jest.fn(), apiWith: jest.fn(), authClient: jest.fn() }));
jest.mock("#/features/session/account", () => ({ currentAccount: jest.fn(), isActive: jest.fn() }));
jest.mock("#/features/session/store", () => ({ useMe: () => ({ timeZone: "Asia/Tokyo" }) }));
jest.mock("expo-router", () => ({
  router: { push: jest.fn(), back: jest.fn() },
  Stack: { Screen: () => null },
}));

const math = create(school.SubjectSchema, {
  id: "s1",
  name: "数学",
  emoji: "📐",
  color: "#4fc3ff",
});
const homework = create(school.AssignmentSchema, {
  id: "a1",
  title: "数学のワーク p.30",
  subject: math,
  dueAt: fromDate(new Date(Date.now() + 86_400_000)),
  remind: true,
  urgency: school.AssignmentUrgency.TOMORROW,
});

/**
 * One account's school services. `switchOnWrite` answers every write only
 * after the person switched to another account; `unreadable` refuses every
 * read (the network is down). What was written is listed.
 */
function schoolOf({ switchOnWrite = false, unreadable = false } = {}) {
  let switched = false;
  const written: string[] = [];
  const write = <T,>(name: string, answer: T) => {
    written.push(name);
    if (switchOnWrite) switched = true;
    return Promise.resolve(answer);
  };
  const day = (date: string) =>
    create(school.TimetableDaySchema, {
      date,
      termId: "t1",
      kind: school.DayKind.SCHOOL_DAY,
      periods: [
        { number: 1, startMinute: 530, endMinute: 580 },
        { number: 2, startMinute: 590, endMinute: 640 },
      ],
      classes: [
        { date, period: 1, subject: math, entryId: "e1", status: school.ClassStatus.SCHEDULED },
        { date, period: 2, subject: math, entryId: "e2", status: school.ClassStatus.CANCELLED },
      ],
    });
  const read =
    <T,>(answer: () => T) =>
    async (..._args: unknown[]) => {
      if (unreadable) throw new Error("offline");
      return answer();
    };
  const client = {
    service: () => ({
      getNow: read(() => ({ now: create(school.ClassNowSchema, {}) })),
      getDay: async ({ date }: { date: string }) => {
        if (unreadable) throw new Error("offline");
        return { day: day(date) };
      },
      listDays: read(() => ({ days: [] })),
      listExams: read(() => ({
        exams: [
          create(school.ExamSchema, {
            id: "x1",
            name: "期末テスト",
            startDate: "2099-12-01",
            status: school.ExamStatus.UPCOMING,
            daysUntilStart: 5,
          }),
        ],
      })),
      listAssignments: read(() => ({ assignments: [homework] })),
      listStudyingFriends: read(() => ({
        friends: [
          create(school.StudyingFriendSchema, {
            sessionId: "f1",
            user: { id: "u2", handle: "mio", displayName: "みお" },
            state: school.SessionState.RUNNING,
            studiedSeconds: 1500,
          }),
        ],
      })),
      updateAssignment: () => write("done", { assignment: homework }),
      createAssignment: () => write("create", { assignment: homework }),
      cheerStudy: () => write("cheer", {}),
    }),
  };
  jest.mocked(api).mockReturnValue(client as never);
  jest
    .mocked(currentAccount)
    .mockReturnValue({ sessionId: "s1", userId: "u1", api: client as never });
  jest.mocked(isActive).mockImplementation(() => !switched);
  return written;
}

async function show(children: ReactNode) {
  await render(
    <SafeAreaProvider
      initialMetrics={{
        frame: { x: 0, y: 0, width: 320, height: 640 },
        insets: { top: 0, left: 0, right: 0, bottom: 0 },
      }}
    >
      <QueryClientProvider
        client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}
      >
        {children}
      </QueryClientProvider>
    </SafeAreaProvider>,
  );
}

beforeEach(() => jest.spyOn(Alert, "alert").mockImplementation(() => {}));
afterEach(() => jest.clearAllMocks());

describe("the school screen", () => {
  it("shows the day's classes, what is due and who is studying", async () => {
    schoolOf();
    await show(<SchoolScreen />);
    expect(await screen.findByTestId("school-day")).toBeTruthy();
    expect(screen.getByText("休講")).toBeTruthy();
    expect(await screen.findByText("数学のワーク p.30")).toBeTruthy();
    expect(screen.getByText("期末テスト")).toBeTruthy();
    expect(await screen.findByText("みお")).toBeTruthy();
  });

  it("says it could not load instead of 「何もないよ」 when nothing can be read", async () => {
    schoolOf({ unreadable: true });
    await show(<SchoolScreen />);
    await waitFor(() => expect(screen.getAllByRole("alert").length).toBe(4));
    expect(screen.queryByTestId("school-due-empty")).toBeNull();
    expect(screen.queryByTestId("school-studying-empty")).toBeNull();
    expect(screen.queryByText("授業の予定はないよ")).toBeNull();
  });

  it("cheers a friend who is studying", async () => {
    const written = schoolOf();
    await show(<SchoolScreen />);
    await fireEvent.press(await screen.findByTestId("cheer-u2"));
    await waitFor(() => expect(written).toEqual(["cheer"]));
    expect(await screen.findByText("送ったよ")).toBeTruthy();
  });
});

describe("assignments", () => {
  it("ticks one off", async () => {
    const written = schoolOf();
    await show(<AssignmentsScreen />);
    await fireEvent.press(await screen.findByTestId("assignment-done-a1"));
    await waitFor(() => expect(written).toEqual(["done"]));
  });

  it("adds one and goes back", async () => {
    const written = schoolOf();
    await show(<NewAssignmentScreen />);
    await fireEvent.changeText(screen.getByTestId("assignment-title"), "英語の単語テスト");
    await fireEvent.press(screen.getByTestId("assignment-due-today"));
    await fireEvent.press(screen.getByTestId("assignment-save"));
    await waitFor(() => expect(written).toEqual(["create"]));
    expect(router.back).toHaveBeenCalled();
  });
});

describe("a school write answered after an account switch", () => {
  it("does not close the form over the next account's screens", async () => {
    const written = schoolOf({ switchOnWrite: true });
    await show(<NewAssignmentScreen />);
    await fireEvent.changeText(screen.getByTestId("assignment-title"), "英語の単語テスト");
    await fireEvent.press(screen.getByTestId("assignment-save"));
    await waitFor(() => expect(written).toEqual(["create"]));
    expect(router.back).not.toHaveBeenCalled();
    expect(Alert.alert).not.toHaveBeenCalled();
  });

  it("does not mark the cheer as sent for the next account", async () => {
    const written = schoolOf({ switchOnWrite: true });
    await show(<SchoolScreen />);
    await fireEvent.press(await screen.findByTestId("cheer-u2"));
    await waitFor(() => expect(written).toEqual(["cheer"]));
    expect(screen.queryByText("送ったよ")).toBeNull();
  });
});
