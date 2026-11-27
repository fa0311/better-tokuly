import { create, type MessageInitShape } from "@bufbuild/protobuf";
import { Code, ConnectError } from "@connectrpc/connect";
import { fromDate, school } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import { testI18n } from "../testing/i18n";
import {
  classWhen,
  currentClass,
  dueItems,
  examCountdown,
  inkOn,
  lastCheerStillCounts,
  takesPlace,
  untilText,
} from "./model";

const scheduled = (init: MessageInitShape<typeof school.ScheduledClassSchema>) =>
  create(school.ScheduledClassSchema, { date: "2026-11-24", period: 2, startMinute: 590, ...init });

describe("classes", () => {
  it("lists cancelled and moved-away classes without counting them as taking place", () => {
    expect(takesPlace(scheduled({ status: school.ClassStatus.SCHEDULED }))).toBe(true);
    expect(takesPlace(scheduled({ status: school.ClassStatus.MOVED_HERE }))).toBe(true);
    expect(takesPlace(scheduled({ status: school.ClassStatus.CANCELLED }))).toBe(false);
    expect(takesPlace(scheduled({ status: school.ClassStatus.MOVED_AWAY }))).toBe(false);
  });

  it("says when a class starts, with the date only for another day", () => {
    const i18n = testI18n("ja");
    expect(classWhen(i18n, scheduled({}), "2026-11-24")).toBe("2限 9:50〜");
    expect(classWhen(i18n, scheduled({}), "2026-11-23")).toBe("11/24(火) 2限 9:50〜");
  });

  it("drops a current class whose bell has rung since the server answered", () => {
    const endsAt = fromDate(new Date(Date.UTC(2026, 10, 24, 1, 40)));
    const now = create(school.ClassNowSchema, { current: scheduled({ endsAt }) });
    expect(currentClass(now, Date.UTC(2026, 10, 24, 1, 39))).toBe(now.current);
    expect(currentClass(now, Date.UTC(2026, 10, 24, 1, 40))).toBeUndefined();
  });

  it("counts down in minutes, then hours", () => {
    const i18n = testI18n("ja");
    expect(untilText(i18n, 61_000)).toBe("あと2分");
    expect(untilText(i18n, 95 * 60_000)).toBe("あと1時間35分");
  });

  it("picks text that reads on a colour", () => {
    expect(inkOn("#c6f24e")).toBe("#17122b");
    expect(inkOn("#3b2a8f")).toBe("#ffffff");
    expect(inkOn("lime")).toBe("#17122b");
  });
});

describe("what is due", () => {
  const exam = (init: MessageInitShape<typeof school.ExamSchema>) =>
    create(school.ExamSchema, { id: "e", name: "期末テスト", ...init });
  const assignment = (id: string, at: Date) =>
    create(school.AssignmentSchema, { id, title: id, dueAt: fromDate(at) });

  it("puts exams and assignments in one list, soonest first, without finished exams", () => {
    const items = dueItems(
      [
        exam({ id: "later", startDate: "2026-12-01", status: school.ExamStatus.UPCOMING }),
        exam({ id: "over", startDate: "2026-11-01", status: school.ExamStatus.FINISHED }),
      ],
      [
        assignment("tomorrow", new Date(Date.UTC(2026, 10, 25, 14, 59))),
        assignment("next-month", new Date(Date.UTC(2026, 11, 2))),
      ],
      "Asia/Tokyo",
    );
    expect(items.map((item) => item.exam?.id ?? item.assignment?.id)).toEqual([
      "tomorrow",
      "later",
      "next-month",
    ]);
  });

  it("counts an exam from midnight of its first day on the account's calendar", () => {
    const [item] = dueItems([exam({ startDate: "2026-12-01" })], [], "Asia/Tokyo");
    expect(item?.at).toBe(Date.UTC(2026, 10, 30, 15, 0));
  });

  it("says how far away an exam is, and how long it still runs", () => {
    const i18n = testI18n("ja");
    const upcoming = { status: school.ExamStatus.UPCOMING, daysLeft: 0 };
    expect(examCountdown(i18n, { ...upcoming, daysUntilStart: 5 })).toBe(
      i18n.t("school:exams.daysUntil", { count: 5 }),
    );
    expect(examCountdown(i18n, { ...upcoming, daysUntilStart: 0 })).toBe(
      i18n.t("school:exams.startsToday"),
    );
    expect(
      examCountdown(i18n, { status: school.ExamStatus.ONGOING, daysUntilStart: 0, daysLeft: 3 }),
    ).toBe(i18n.t("school:exams.ongoing", { count: 3 }));
  });
});

describe("cheers", () => {
  const refused = (reason: string) =>
    new ConnectError("", Code.ResourceExhausted, { "x-tokuly-error": reason });

  it("counts the last cheer only when the friend's cool-down refused this one", () => {
    expect(lastCheerStillCounts(refused("CHEER_TOO_SOON"))).toBe(true);
    // The sender's own limit: nothing was sent to this friend.
    expect(lastCheerStillCounts(refused("RATE_LIMITED"))).toBe(false);
    expect(lastCheerStillCounts(new Error("offline"))).toBe(false);
  });
});
