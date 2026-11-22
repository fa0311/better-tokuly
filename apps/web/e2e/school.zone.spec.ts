import { type Account, befriend, createAccount, expect, rpc, test } from "./support";

const TIMETABLE = "tokuly.school.v1.TimetableService";
const ASSIGNMENTS = "tokuly.school.v1.AssignmentService";
const WEEKDAYS = [
  "WEEKDAY_MONDAY",
  "WEEKDAY_TUESDAY",
  "WEEKDAY_WEDNESDAY",
  "WEEKDAY_THURSDAY",
  "WEEKDAY_FRIDAY",
  "WEEKDAY_SATURDAY",
  "WEEKDAY_SUNDAY",
];

const tokyo = (options: Intl.DateTimeFormatOptions, locale = "ja-JP") =>
  new Intl.DateTimeFormat(locale, { timeZone: "Asia/Tokyo", ...options });

/**
 * A device time zone in which it is a different calendar day than in Tokyo
 * (the accounts' zone) right now: Honolulu is nineteen hours behind, so it
 * is still yesterday there until 19:00 in Tokyo; after that, Kiritimati,
 * five hours ahead, is already in tomorrow.
 */
const tokyoHour = Number(tokyo({ hour: "numeric", hourCycle: "h23" }, "en-US").format(new Date()));
const DEVICE_ZONE = tokyoHour < 19 ? "Pacific/Honolulu" : "Pacific/Kiritimati";

test.use({ timezoneId: DEVICE_ZONE });

async function subject(
  request: Parameters<typeof rpc>[0],
  as: Account,
  name: string,
): Promise<string> {
  return (
    await rpc<{ subject: { id: string } }>(request, `${TIMETABLE}/CreateSubject`, { name }, as)
  ).subject.id;
}

test("today is the account's day, not the device's, when the device is in another time zone", async ({
  page,
  request,
  me,
}) => {
  const now = new Date();
  const tokyoDate = tokyo({}, "sv-SE").format(now);
  const [, month, day] = tokyoDate.split("-").map(Number);
  const tokyoLabel = `${month}/${day}(${tokyo({ weekday: "short" }).format(now)})`;
  const tokyoWeekday = (new Date(`${tokyoDate}T00:00:00Z`).getUTCDay() + 6) % 7;

  // The premise: the browser's own calendar is on another day than the account's.
  await page.goto("/school");
  expect(
    await page.evaluate(
      () =>
        `${new Date().getFullYear()}-${String(new Date().getMonth() + 1).padStart(2, "0")}-${String(new Date().getDate()).padStart(2, "0")}`,
    ),
  ).not.toBe(tokyoDate);

  // The first period is "きょうの教科" on the account's weekday and "べつの日" on every other day.
  const todays = await subject(request, me, "きょうの教科");
  const others = await subject(request, me, "べつの日");
  const { term } = await rpc<{ term: { id: string } }>(
    request,
    `${TIMETABLE}/CreateTerm`,
    {
      name: "タイムゾーン",
      startDate: tokyo({}, "sv-SE").format(new Date(now.getTime() - 30 * 86_400_000)),
      endDate: tokyo({}, "sv-SE").format(new Date(now.getTime() + 60 * 86_400_000)),
      layout: { preset: "LAYOUT_PRESET_HIGH_SCHOOL_6", weekdays: WEEKDAYS },
      observePublicHolidays: false,
    },
    me,
  );
  await rpc(
    request,
    `${TIMETABLE}/PutTimetableEntries`,
    {
      termId: term.id,
      entries: WEEKDAYS.map((weekday, index) => ({
        weekday,
        period: 1,
        subjectId: index === tokyoWeekday ? todays : others,
      })),
    },
    me,
  );

  // /school opens on the account's date and lists that day's classes.
  await page.reload();
  await expect(page.getByRole("heading", { name: "今日の授業" })).toBeVisible();
  await expect(
    page.getByRole("list", { name: "この週" }).getByRole("button", { pressed: true }),
  ).toHaveAccessibleName(new RegExp(`^${tokyoLabel.replace(/[()]/g, "\\$&")}`));
  const classes = page.getByRole("list", { name: /の授業$/ });
  await expect(classes).toHaveAccessibleName(`${tokyoLabel} の授業`);
  await expect(classes.getByRole("listitem").nth(0)).toContainText("きょうの教科");

  // The grid marks the account's weekday as today.
  await page.goto("/school/timetable");
  const headers = page.getByRole("table", { name: "時間割" }).getByRole("columnheader");
  await expect(headers.nth(tokyoWeekday)).toHaveAttribute("aria-current", "date");
  await expect(
    page.getByRole("table", { name: "時間割" }).locator('[aria-current="date"]'),
  ).toHaveCount(1);

  // Free periods start from the account's today.
  const friend = await createAccount(request, "zonefriend");
  await befriend(request, me, friend);
  await page.goto("/school/friends?tab=free");
  await expect(page.getByRole("heading", { name: `今日 ${tokyoLabel}` })).toBeVisible();

  // A new assignment is due at the end of the account's tomorrow, and times are entered on the account's clock.
  await page.goto("/school/assignments");
  await page.getByRole("button", { name: "課題を追加" }).first().click();
  const dialog = page.getByRole("dialog");
  const tomorrow = tokyo({}, "sv-SE").format(new Date(now.getTime() + 86_400_000));
  await expect(dialog.getByLabel("しめきり", { exact: true })).toHaveValue(`${tomorrow}T23:59`);
  await dialog.getByLabel("やること").fill("時差の宿題");
  await dialog.getByRole("button", { name: "追加する" }).click();
  await expect(page.getByText("時差の宿題")).toBeVisible();
  const { assignments } = await rpc<{ assignments: { dueAt: string; urgency: string }[] }>(
    request,
    `${ASSIGNMENTS}/ListAssignments`,
    {},
    me,
  );
  expect(
    tokyo({ dateStyle: "short", timeStyle: "short" }, "sv-SE").format(
      new Date(assignments[0]?.dueAt ?? 0),
    ),
  ).toBe(`${tomorrow} 23:59`);
  expect(assignments[0]?.urgency).toBe("ASSIGNMENT_URGENCY_TOMORROW");
  await expect(page.getByRole("listitem").filter({ hasText: "時差の宿題" })).toContainText(
    "23:59 まで",
  );
});
