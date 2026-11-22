import type { APIRequestContext, Browser, Page, TestInfo } from "@playwright/test";
import { type Account, befriend, createAccount, expect, rpc, signInAs, test } from "./support";

const TIMETABLE = "tokuly.school.v1.TimetableService";
const STUDY = "tokuly.school.v1.StudyService";
const EXAMS = "tokuly.school.v1.ExamService";
const ASSIGNMENTS = "tokuly.school.v1.AssignmentService";

const ALL_WEEKDAYS = ["WEEKDAY_MONDAY", "WEEKDAY_TUESDAY", "WEEKDAY_WEDNESDAY", "WEEKDAY_THURSDAY", "WEEKDAY_FRIDAY", "WEEKDAY_SATURDAY", "WEEKDAY_SUNDAY"];

/** The calendar date in Tokyo (the accounts' time zone), `days` from today. */
function tokyoDate(days = 0): string {
  return new Intl.DateTimeFormat("sv-SE", { timeZone: "Asia/Tokyo" }).format(new Date(Date.now() + days * 86_400_000));
}

/** A moment as the value of a `datetime-local` input in Tokyo. */
function tokyoInput(date: Date): string {
  return new Intl.DateTimeFormat("sv-SE", { timeZone: "Asia/Tokyo", dateStyle: "short", timeStyle: "short" }).format(date).replace(" ", "T");
}

/** Today's weekday in Tokyo as the grid names it: "月曜日". */
function tokyoWeekday(days = 0): string {
  return new Intl.DateTimeFormat("ja-JP", { timeZone: "Asia/Tokyo", weekday: "long" }).format(new Date(Date.now() + days * 86_400_000));
}

async function createSubject(request: APIRequestContext, as: Account, name: string): Promise<string> {
  const { subject } = await rpc<{ subject: { id: string } }>(request, `${TIMETABLE}/CreateSubject`, { name }, as);
  return subject.id;
}

/**
 * A term that has classes every day of the week (so "today" always has some)
 * with `subjectId` in the given periods of every weekday.
 */
async function createTerm(request: APIRequestContext, as: Account, options: { name: string; subjectId: string; periods: number[]; audience?: string }): Promise<string> {
  const { term } = await rpc<{ term: { id: string } }>(
    request,
    `${TIMETABLE}/CreateTerm`,
    {
      name: options.name,
      startDate: tokyoDate(-30),
      endDate: tokyoDate(90),
      layout: { preset: "LAYOUT_PRESET_HIGH_SCHOOL_6", weekdays: ALL_WEEKDAYS },
      observePublicHolidays: false,
      audience: options.audience ?? "TIMETABLE_AUDIENCE_NOBODY",
    },
    as,
  );
  await rpc(request, `${TIMETABLE}/PutTimetableEntries`, { termId: term.id, entries: ALL_WEEKDAYS.flatMap((weekday) => options.periods.map((period) => ({ weekday, period, subjectId: options.subjectId }))) }, as);
  return term.id;
}

/** A second person's browser, signed in, at the size of the running project. */
async function browserOf(browser: Browser, testInfo: TestInfo, account: Account): Promise<Page> {
  const context = await browser.newContext({ baseURL: testInfo.project.use.baseURL, locale: "ja-JP", timezoneId: "Asia/Tokyo", viewport: testInfo.project.use.viewport, hasTouch: testInfo.project.use.hasTouch });
  const page = await context.newPage();
  await signInAs(page, account);
  return page;
}

/** The seconds a timer face shows ("24:57" → 1497). */
async function timerSeconds(page: Page): Promise<number> {
  const text = (await page.getByRole("timer").innerText()).trim();
  return text.split(":").reduce((total, part) => total * 60 + Number(part), 0);
}

// ───────────────────────────── Timetable ─────────────────────────────

test("a timetable is built by tapping slots, and today shows its classes and a one-off change", async ({ page, me: _me }) => {
  await page.goto("/me");
  await page.getByRole("main").getByRole("link", { name: /スクール/ }).click();
  await expect(page).toHaveURL(/\/school$/);

  await page.getByRole("link", { name: "時間割", exact: true }).click();
  await expect(page.getByText("時間割をつくろう!")).toBeVisible();

  // A term with classes on every day of the week, so today has some whatever day it is.
  await page.getByRole("button", { name: "新しい学期" }).click();
  const termDialog = page.getByRole("dialog");
  await termDialog.getByLabel("学期のなまえ").fill("テスト学期");
  await termDialog.getByRole("button", { name: "土曜日" }).click();
  await termDialog.getByRole("button", { name: "日曜日" }).click();
  await termDialog.getByRole("switch", { name: /祝日は休みにする/ }).click();
  await termDialog.getByRole("button", { name: "つくる" }).click();
  await expect(page.getByRole("heading", { name: "テスト学期" })).toBeVisible();

  // Edit mode: add a subject once, then tap several slots with it.
  await page.getByRole("button", { name: "編集", exact: true }).click();
  await page.getByRole("button", { name: "教科を追加" }).click();
  const subjectDialog = page.getByRole("dialog");
  await subjectDialog.getByLabel("なまえ", { exact: true }).fill("数学");
  await subjectDialog.getByLabel("教室").fill("2-A");
  await subjectDialog.getByRole("button", { name: "保存する" }).click();
  await expect(page.getByRole("radio", { name: "数学" })).toBeChecked();

  const today = tokyoWeekday();
  const tomorrow = tokyoWeekday(1);
  const grid = page.getByRole("table", { name: "時間割" });
  for (const slot of [`${today} 1限`, `${today} 2限`, `${tomorrow} 1限`, `${tomorrow} 3限`]) {
    await grid.getByRole("button", { name: `${slot} 空き` }).click();
    await expect(grid.getByRole("button", { name: `${slot} 数学` })).toBeVisible();
  }
  // Tapping a class again with the same subject clears the slot.
  await grid.getByRole("button", { name: `${tomorrow} 3限 数学` }).click();
  await expect(grid.getByRole("button", { name: `${tomorrow} 3限 空き` })).toBeVisible();
  await page.getByRole("button", { name: "完了" }).click();

  // Today: the widget names the class that is on or next, and the day lists both classes.
  await page.getByRole("link", { name: "きょう", exact: true }).click();
  await expect(page.getByRole("region", { name: "いまの授業" })).toContainText("数学");
  const day = page.getByRole("list", { name: /の授業$/ });
  await expect(day.getByRole("listitem").nth(0)).toContainText("数学");
  await expect(day.getByRole("listitem").nth(0)).toContainText("2-A");
  await expect(day.getByRole("listitem").nth(1)).toContainText("数学");
  await expect(day.getByRole("listitem").nth(2)).toContainText("空きコマ");

  // A one-off change: the second period is cancelled today only.
  await day.getByRole("button", { name: "2限を変更する" }).click();
  await page.getByRole("dialog").getByRole("button", { name: "変更を入れる" }).click();
  await expect(day.getByRole("listitem").nth(1)).toContainText("休講");
  await page.goto("/school/timetable");
  await expect(page.getByRole("table", { name: "時間割" }).getByRole("button", { name: `${today} 2限 数学` })).toBeVisible();
});

test("a shared timetable shows up for a friend, who copies it as a starting point", async ({ page, request, browser, me }, testInfo) => {
  const friend = await createAccount(request, "classmate");
  await befriend(request, me, friend);
  await createTerm(request, me, { name: "2年後期", subjectId: await createSubject(request, me, "世界史"), periods: [1, 2] });

  // Private at first: the friend sees nothing.
  const theirs = await browserOf(browser, testInfo, friend);
  await theirs.goto("/school/friends");
  await expect(theirs.getByText("まだだれも時間割を共有していないよ")).toBeVisible();

  // Share it with every friend.
  await page.goto("/school/timetable");
  await page.getByRole("button", { name: "自分だけ" }).click();
  const share = page.getByRole("dialog");
  await share.getByRole("radio", { name: "ともだち全員" }).click();
  await share.getByRole("button", { name: "保存する" }).click();
  await expect(page.getByRole("button", { name: "ともだち全員" })).toBeVisible();

  // The friend finds it, looks at it, and copies it.
  await theirs.reload();
  await theirs.getByRole("link", { name: new RegExp(me.displayName) }).click();
  await expect(theirs.getByRole("table", { name: "時間割" })).toContainText("世界史");
  await theirs.getByRole("button", { name: "コピーする" }).click();
  const copy = theirs.getByRole("dialog");
  await copy.getByLabel("学期のなまえ").fill("わたしの時間割");
  await copy.getByRole("button", { name: "コピーする" }).click();

  await expect(theirs).toHaveURL(/\/school\/timetable\?term=/);
  await expect(theirs.getByRole("heading", { name: "わたしの時間割" })).toBeVisible();
  await expect(theirs.getByRole("table", { name: "時間割" }).getByRole("button", { name: `${tokyoWeekday()} 1限 世界史` })).toBeVisible();
  await expect(theirs.getByText(`${me.displayName}さんからコピー`)).toBeVisible();
  // The copy is theirs: the subject was added to their own list.
  const { subjects } = await rpc<{ subjects: { name: string }[] }>(request, `${TIMETABLE}/ListSubjects`, {}, friend);
  expect(subjects.map((subject) => subject.name)).toContain("世界史");
  await theirs.context().close();
});

test("free periods list the friends who are free at the same time", async ({ page, request, me }) => {
  const friend = await createAccount(request, "freebird");
  await befriend(request, me, friend);
  // Me: classes in periods 1 and 3, so period 2 is a gap between classes. The friend: period 1 only.
  await createTerm(request, me, { name: "わたし", subjectId: await createSubject(request, me, "化学"), periods: [1, 3] });
  await createTerm(request, friend, { name: "ともだち", subjectId: await createSubject(request, friend, "体育"), periods: [1], audience: "TIMETABLE_AUDIENCE_FRIENDS" });

  await page.goto("/school/timetable");
  await page.getByRole("link", { name: /空きコマ/ }).click();
  await expect(page).toHaveURL(/\/school\/friends\?tab=free/);

  const today = page.locator("section").filter({ has: page.getByRole("heading", { name: /^今日/ }) });
  const second = today.getByRole("listitem").filter({ hasText: "2限" });
  await expect(second).toContainText(friend.displayName);
  await expect(second).toContainText("授業のあいだ");
  // Periods with a class of mine are not free periods.
  await expect(today.getByRole("listitem").filter({ hasText: "1限" })).toHaveCount(0);
  await expect(today.getByRole("listitem").filter({ hasText: "4限" })).toContainText(friend.displayName);

  // The friends' tab says what the friend is in right now, and opens their timetable.
  await page.getByRole("tab", { name: "時間割" }).click();
  await page.getByRole("link", { name: new RegExp(friend.displayName) }).click();
  await expect(page.getByRole("table", { name: "時間割" })).toContainText("体育");
});

// ───────────────────────────── Study ─────────────────────────────

test("the study timer runs on timestamps and survives a reload; logged time shows in totals, heatmap and log", async ({ page, request, me }) => {
  await createSubject(request, me, "英語");
  await page.clock.install();
  await page.goto("/school/study");

  const timer = page.getByRole("region", { name: "勉強タイマー" });
  await timer.getByRole("combobox").click();
  await page.getByRole("option", { name: "英語" }).click();
  await timer.getByRole("button", { name: "スタート!" }).click();
  await expect(timer.getByRole("timer")).toHaveText(/^00:0\d$/);

  // Three quarters of a minute pass without a single tick being counted: the face follows the clock.
  await page.clock.fastForward("00:45");
  await expect(timer.getByRole("timer")).toHaveText(/^00:(4[5-9]|5\d)$/);

  // After a reload the timer is still running (the server keeps it), on the same subject.
  await page.reload();
  await expect(timer.getByRole("button", { name: "おわる" })).toBeVisible();
  await expect(timer).toContainText("英語");
  expect((await rpc<{ session?: { state: string } }>(request, `${STUDY}/GetCurrentStudySession`, {}, me)).session?.state).toBe("SESSION_STATE_RUNNING");

  await timer.getByRole("button", { name: "一時停止" }).click();
  await expect(timer.getByText("一時停止中")).toBeVisible();
  await timer.getByRole("button", { name: "再開" }).click();
  await timer.getByRole("button", { name: "おわる" }).click();
  await page.getByRole("dialog").getByLabel("メモ").fill("単語 100個");
  await page.getByRole("dialog").getByRole("button", { name: "記録しておわる" }).click();
  await expect(timer.getByRole("button", { name: "スタート!" })).toBeVisible();

  // The timed session is in the log, labelled by its source.
  const log = page.getByRole("list", { name: "勉強の記録" });
  await expect(log.getByRole("listitem").filter({ hasText: "タイマー" })).toContainText("単語 100個");

  // An hour logged by hand afterwards.
  await page.getByRole("button", { name: "あとから記録" }).click();
  const dialog = page.getByRole("dialog");
  await dialog.getByLabel("はじめた時間").fill(tokyoInput(new Date(Date.now() - 65 * 60_000)));
  await dialog.getByLabel("おわった時間").fill(tokyoInput(new Date(Date.now() - 5 * 60_000)));
  await expect(dialog).toContainText("ランキングには入らないよ");
  await dialog.getByRole("button", { name: "記録する" }).click();

  const manual = log.getByRole("listitem").filter({ hasText: "手入力" });
  await expect(manual).toContainText("1時間");
  // It counts in one's own totals and colours the calendar.
  const progress = page.getByRole("region", { name: "今日のようす" });
  await expect(progress).toContainText(/1時間.*これまでの合計/s);
  const heatmap = page.locator("section").filter({ has: page.getByRole("heading", { name: "勉強カレンダー" }) });
  await expect(heatmap).toContainText("この1年で 1時間");
  await expect(heatmap.locator('[data-level="4"]')).not.toHaveCount(0);

  // The ranking counts only time the server timed: the hand-logged hour is not in it.
  await page.getByRole("button", { name: "ランキングに参加する" }).click();
  const mine = page.getByRole("list", { name: "今週のランキング" }).getByRole("listitem").filter({ hasText: "あなた" });
  await expect(mine).toBeVisible();
  await expect(mine).not.toContainText("1時間");

  // A timed session can only be made shorter; the dialog says so.
  await log.getByRole("listitem").filter({ hasText: "タイマー" }).getByRole("button").click();
  await expect(page.getByRole("dialog")).toContainText("短くすることだけできるよ");
});

test("a friend who is studying shows on their profile and can be cheered on; an exam period shows too", async ({ page, request, me }) => {
  const friend = await createAccount(request, "bookworm");
  await befriend(request, me, friend);
  await rpc(request, `${STUDY}/StartStudySession`, { mode: "STUDY_MODE_STOPWATCH", subjectId: await createSubject(request, friend, "古典") }, friend);
  await rpc(request, `${EXAMS}/CreateExam`, { name: "期末テスト", startDate: tokyoDate(5), endDate: tokyoDate(8), showOnProfile: true }, friend);

  await page.goto(`/u/${friend.handle}`);
  const studying = page.locator('[data-kind="school.studying"]');
  await expect(studying).toContainText("勉強中");
  await expect(studying).toContainText("古典");
  await expect(page.locator('[data-kind="school.exam"]')).toContainText("期末テスト");

  await studying.getByRole("button", { name: /がんばれ/ }).click();
  await expect(page.getByText(`${friend.displayName}さんに「がんばれ!」を送ったよ`)).toBeVisible();
  const { session } = await rpc<{ session: { cheerCount?: number } }>(request, `${STUDY}/GetCurrentStudySession`, {}, friend);
  expect(session.cheerCount).toBe(1);

  // The study page lists the friend too.
  await page.goto("/school/study");
  await expect(page.locator("section").filter({ has: page.getByRole("heading", { name: "勉強中のともだち" }) })).toContainText(friend.displayName);
});

// ───────────────────────────── Study rooms ─────────────────────────────

test("two people in a study room follow one timer, see each other and react", async ({ page, request, browser, me }, testInfo) => {
  const friend = await createAccount(request, "roomie");
  await befriend(request, me, friend);

  // The host opens a room and is inside at once; the timer waits.
  await page.goto("/school/rooms");
  await page.getByRole("button", { name: "部屋をつくる" }).first().click();
  await page.getByRole("dialog").getByLabel("部屋のなまえ").fill("テスト前もくもく");
  await page.getByRole("dialog").getByRole("button", { name: "つくって入る" }).click();
  await expect(page).toHaveURL(/\/school\/rooms\/.+/);
  await expect(page.getByRole("heading", { name: "テスト前もくもく" })).toBeVisible();
  await expect(page.getByText("スタート待ち")).toBeVisible();

  // The friend finds the room among friends' rooms and enters.
  const theirs = await browserOf(browser, testInfo, friend);
  await theirs.goto("/school/rooms");
  await theirs.getByRole("link", { name: /テスト前もくもく/ }).click();
  await theirs.getByRole("button", { name: "入る" }).click();
  await expect(theirs.getByText(`${me.displayName}さんがタイマーをスタートするのを待っているよ`)).toBeVisible();

  // Presence, on both screens.
  const members = page.getByRole("list", { name: /^いる人/ });
  await expect(members.getByRole("listitem")).toHaveCount(2);
  await expect(members).toContainText(friend.displayName);
  await expect(theirs.getByRole("list", { name: /^いる人/ })).toContainText(me.displayName);

  // The host starts the timer: both follow the same countdown.
  await page.getByRole("button", { name: "タイマーをスタート" }).click();
  await expect(page.getByText("集中", { exact: false }).first()).toBeVisible();
  await expect(theirs.getByRole("region", { name: "みんなのタイマー" })).toContainText("集中");
  await expect(async () => {
    const [mine, other] = await Promise.all([timerSeconds(page), timerSeconds(theirs)]);
    expect(mine).toBeLessThan(25 * 60);
    expect(Math.abs(mine - other)).toBeLessThanOrEqual(2);
  }).toPass();

  // A reaction from the friend arrives at the host.
  await theirs.getByRole("button", { name: "がんばれ!", exact: true }).click();
  await expect(page.getByText(`${friend.displayName}: がんばれ!`)).toBeVisible();

  // The host pauses: the friend's timer stops with it.
  await page.getByRole("button", { name: "一時停止" }).click();
  await expect(theirs.getByRole("region", { name: "みんなのタイマー" })).toContainText("一時停止中");
  const paused = await timerSeconds(theirs);
  await theirs.waitForTimeout(1500);
  expect(await timerSeconds(theirs)).toBe(paused);

  // The friend leaves; the host sees them go.
  await theirs.getByRole("button", { name: "部屋を出る" }).click();
  await expect(theirs).toHaveURL(/\/school\/rooms$/);
  await expect(members.getByRole("listitem")).toHaveCount(1);
  await theirs.context().close();
});

// ───────────────────────────── Exams and assignments ─────────────────────────────

test("an exam counts down, an assignment gets reminders, and both are in the list of what is due", async ({ page, request, me }) => {
  await page.goto("/school/exams");

  await page.getByRole("button", { name: "テストを追加" }).click();
  const examDialog = page.getByRole("dialog");
  await examDialog.getByLabel("なまえ").fill("中間テスト");
  await examDialog.getByLabel("はじまる日").fill(tokyoDate(10));
  await examDialog.getByLabel(/^おわる日/).fill(tokyoDate(12));
  await examDialog.getByRole("button", { name: "追加する" }).click();
  await expect(page).toHaveURL(/\/school\/exams\/.+/);
  await expect(page.getByRole("heading", { name: "中間テスト" })).toBeVisible();
  await expect(page.getByText("あと10日")).toBeVisible();

  await page.getByRole("link", { name: "back" }).click();
  await page.getByRole("button", { name: "課題を追加" }).click();
  const assignmentDialog = page.getByRole("dialog");
  await assignmentDialog.getByLabel("やること").fill("英語のプリント");
  await expect(assignmentDialog.getByRole("switch", { name: /リマインドする/ })).toBeChecked();
  await assignmentDialog.getByRole("button", { name: "追加する" }).click();
  await expect(page.getByText("しめきり前にお知らせするね!")).toBeVisible();

  // One list of what is due: the assignment (due tomorrow) before the exam (in ten days).
  const due = page.getByRole("list", { name: "しめきり" });
  await expect(due.getByRole("listitem").nth(0)).toContainText("英語のプリント");
  await expect(due.getByRole("listitem").nth(0)).toContainText("明日まで");
  await expect(due.getByRole("listitem").nth(1)).toContainText("中間テスト");
  await expect(due.getByRole("listitem").nth(1)).toContainText("あと10日");
  const { assignments } = await rpc<{ assignments: { title: string; remind?: boolean }[] }>(request, `${ASSIGNMENTS}/ListAssignments`, {}, me);
  expect(assignments).toMatchObject([{ title: "英語のプリント", remind: true }]);

  // Ticking it off takes it out of what is due and into the finished ones.
  await due.getByRole("button", { name: "「英語のプリント」を完了にする" }).click();
  await expect(due.getByText("英語のプリント")).toHaveCount(0);
  await page.getByRole("link", { name: "課題をぜんぶ見る" }).click();
  await page.getByRole("tab", { name: "おわった" }).click();
  await expect(page.getByText("英語のプリント")).toBeVisible();
});
