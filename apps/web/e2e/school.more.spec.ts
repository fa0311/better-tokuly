import type { APIRequestContext } from "@playwright/test";
import { type Account, befriend, createAccount, expect, rpc, test } from "./support";

const TIMETABLE = "tokuly.school.v1.TimetableService";
const ROOMS = "tokuly.school.v1.StudyRoomService";
const EXAMS = "tokuly.school.v1.ExamService";
const SETTINGS = "tokuly.school.v1.SchoolService";

const ALL_WEEKDAYS = [
  "WEEKDAY_MONDAY",
  "WEEKDAY_TUESDAY",
  "WEEKDAY_WEDNESDAY",
  "WEEKDAY_THURSDAY",
  "WEEKDAY_FRIDAY",
  "WEEKDAY_SATURDAY",
  "WEEKDAY_SUNDAY",
];

/** The calendar date in Tokyo (the accounts' time zone), `days` from today. */
function tokyoDate(days = 0): string {
  return new Intl.DateTimeFormat("sv-SE", { timeZone: "Asia/Tokyo" }).format(
    new Date(Date.now() + days * 86_400_000),
  );
}

function tokyoWeekday(): string {
  return new Intl.DateTimeFormat("ja-JP", { timeZone: "Asia/Tokyo", weekday: "long" }).format(
    new Date(),
  );
}

async function createSubject(
  request: APIRequestContext,
  as: Account,
  name: string,
): Promise<string> {
  return (
    await rpc<{ subject: { id: string } }>(request, `${TIMETABLE}/CreateSubject`, { name }, as)
  ).subject.id;
}

test("a slot holds one class for week A and another for week B, with a private memo; a day off empties the day", async ({
  page,
  request,
  me,
}) => {
  await createSubject(request, me, "数学");
  await createSubject(request, me, "英語");
  const { term } = await rpc<{ term: { id: string } }>(
    request,
    `${TIMETABLE}/CreateTerm`,
    {
      name: "AB学期",
      startDate: tokyoDate(-30),
      endDate: tokyoDate(90),
      layout: { preset: "LAYOUT_PRESET_HIGH_SCHOOL_6", weekdays: ALL_WEEKDAYS },
      observePublicHolidays: false,
      alternatingWeeks: true,
      weekADate: tokyoDate(),
    },
    me,
  );

  await page.goto("/school/timetable");
  const grid = page.getByRole("table", { name: "時間割" });
  const slot = `${tokyoWeekday()} 1限`;
  await page.getByRole("button", { name: "編集", exact: true }).click();
  await page.getByRole("radio", { name: "数学" }).click();
  await page.getByRole("radio", { name: "A週" }).click();
  await grid.getByRole("button", { name: `${slot} 空き` }).click();
  await page.getByRole("radio", { name: "英語" }).click();
  await page.getByRole("radio", { name: "B週" }).click();
  await grid.getByRole("button", { name: `${slot} A週 数学` }).click();
  await expect(grid.getByRole("button", { name: `${slot} A週 数学 / B週 英語` })).toBeVisible();
  await page.getByRole("button", { name: "完了" }).click();

  // Looking at the slot: each week's class has its own room and memo.
  await grid.getByRole("button", { name: `${slot} A週 数学 / B週 英語` }).click();
  const dialog = page.getByRole("dialog");
  await dialog.getByRole("radio", { name: "B週" }).click();
  await dialog.getByLabel("教室").fill("LL教室");
  await dialog.getByLabel("メモ").fill("小テストあり");
  await dialog.getByRole("button", { name: "保存する" }).click();
  await expect(dialog).toBeHidden();
  await expect(async () => {
    const { entries } = await rpc<{
      entries: { weekPattern: string; room?: string; memo?: string }[];
    }>(request, `${TIMETABLE}/GetTerm`, { termId: term.id }, me);
    expect(entries).toHaveLength(2);
    expect(entries.find((entry) => entry.weekPattern === "WEEK_PATTERN_B")).toMatchObject({
      room: "LL教室",
      memo: "小テストあり",
    });
  }).toPass();

  // Today runs as week A (the term says so): the day shows the week-A class.
  await page.goto("/school");
  const day = page.getByRole("list", { name: /の授業$/ });
  await expect(day.getByRole("listitem").nth(0)).toContainText("数学");
  await expect(page.getByText("A週", { exact: true })).toBeVisible();

  // A day off for today: no classes any more.
  await page.goto("/school/timetable");
  await page.getByRole("button", { name: "時間割のメニュー" }).click();
  await page.getByRole("menuitem", { name: "休み・登校日" }).click();
  await page.getByRole("dialog").getByLabel("なまえ").fill("創立記念日");
  await page.getByRole("dialog").getByRole("button", { name: "追加", exact: true }).click();
  await expect(
    page.getByRole("dialog").getByRole("button", { name: "創立記念日を消す" }),
  ).toBeVisible();
  await page.goto("/school");
  await expect(page.getByText("創立記念日")).toBeVisible();
  await expect(page.getByText("授業はありません。ゆっくりしよう!")).toBeVisible();
});

test("the host of a study room invites, renames, removes someone and closes the room", async ({
  page,
  request,
  me,
}) => {
  const friend = await createAccount(request, "invitee");
  await befriend(request, me, friend);
  const { room } = await rpc<{ room: { id: string } }>(
    request,
    `${ROOMS}/CreateStudyRoom`,
    { title: "招待だけの部屋", audience: "ROOM_AUDIENCE_INVITED" },
    me,
  );

  await page.goto(`/school/rooms/${room.id}`);
  await expect(page.getByRole("heading", { name: "招待だけの部屋" })).toBeVisible();
  // The host is inside already (opening a room enters it) and the timer is theirs to start.
  await expect(page.getByRole("button", { name: "タイマーをスタート" })).toBeVisible();

  await page.getByRole("button", { name: "部屋のメニュー" }).click();
  await page.getByRole("menuitem", { name: "ともだちを招待" }).click();
  await page.getByRole("dialog").getByRole("button", { name: friend.displayName }).click();
  await page.getByRole("dialog").getByRole("button", { name: "招待する" }).click();
  await expect(
    page
      .getByRole("dialog")
      .getByRole("button", { name: `${friend.displayName}さんの招待を取り消す` }),
  ).toBeVisible();
  await page.getByRole("dialog").getByRole("button", { name: "close" }).click();

  // The invitation lets the friend in; the host sees them arrive, then removes them.
  await rpc(request, `${ROOMS}/JoinStudyRoom`, { roomId: room.id }, friend);
  const members = page.getByRole("list", { name: /^いる人/ });
  await expect(members.getByRole("listitem")).toHaveCount(2);
  await members.getByRole("button", { name: new RegExp(friend.displayName) }).click();
  await page.getByRole("dialog").getByRole("button", { name: "部屋から出てもらう" }).click();
  await expect(members.getByRole("listitem")).toHaveCount(1);

  await page.getByRole("button", { name: "部屋のメニュー" }).click();
  await page.getByRole("menuitem", { name: "部屋の設定" }).click();
  await page.getByRole("dialog").getByLabel("部屋のなまえ").fill("みんなの部屋");
  await page.getByRole("dialog").getByRole("radio", { name: "だれでも" }).click();
  await page.getByRole("dialog").getByRole("button", { name: "保存する" }).click();
  await expect(page.getByRole("heading", { name: "みんなの部屋" })).toBeVisible();

  await page.getByRole("button", { name: "部屋のメニュー" }).click();
  await page.getByRole("menuitem", { name: "部屋を閉じる" }).click();
  await page.getByRole("dialog").getByRole("button", { name: "部屋を閉じる" }).click();
  await expect(page).toHaveURL(/\/school\/rooms$/);
  await expect(page.getByText("部屋を閉じたよ。おつかれさま!")).toBeVisible();
  await expect(page.getByText("みんなの部屋")).toHaveCount(0);
});

test("an exam gets its subjects, scope and a private result; the daily goal shows on the study page", async ({
  page,
  request,
  me,
}) => {
  await createSubject(request, me, "物理");
  const { exam } = await rpc<{ exam: { id: string } }>(
    request,
    `${EXAMS}/CreateExam`,
    { name: "学年末テスト", startDate: tokyoDate(3), endDate: tokyoDate(5) },
    me,
  );

  await page.goto(`/school/exams/${exam.id}`);
  await page.getByRole("button", { name: "教科を追加" }).first().click();
  const subjectDialog = page.getByRole("dialog");
  await subjectDialog.getByRole("combobox").click();
  await page.getByRole("option", { name: "物理" }).click();
  await subjectDialog.getByLabel("テストの日").fill(tokyoDate(4));
  await subjectDialog.getByLabel("テスト範囲").fill("力学 p.10〜60");
  await subjectDialog.getByRole("button", { name: "保存する" }).click();
  await expect(page.getByText("力学 p.10〜60")).toBeVisible();

  await page.getByRole("button", { name: "結果をメモ" }).click();
  const resultDialog = page.getByRole("dialog");
  await resultDialog.getByLabel("点数").fill("82");
  await resultDialog.getByLabel("順位").fill("12");
  await resultDialog.getByRole("button", { name: "保存する" }).click();
  await expect(page.getByText("82 / 100")).toBeVisible();
  await expect(page.getByText("12位")).toBeVisible();

  // Showing the exam period on the profile is a switch on the exam.
  await page.getByRole("switch", { name: /プロフィールに「テスト期間」を出す/ }).click();
  await expect(async () => {
    expect(
      (
        await rpc<{ exam: { showOnProfile?: boolean } }>(
          request,
          `${EXAMS}/GetExam`,
          { examId: exam.id },
          me,
        )
      ).exam.showOnProfile,
    ).toBe(true);
  }).toPass();

  // Settings: a daily goal, saved when the field is left.
  await page.goto("/school/settings");
  await page.getByLabel("1日の目標（分）").fill("45");
  await page.getByLabel("1日の目標（分）").blur();
  await expect(async () => {
    expect(
      (
        await rpc<{ settings: { dailyGoalMinutes?: number } }>(
          request,
          `${SETTINGS}/GetSchoolSettings`,
          {},
          me,
        )
      ).settings.dailyGoalMinutes,
    ).toBe(45);
  }).toPass();
  await page.goto("/school/study");
  await expect(page.getByRole("region", { name: "今日のようす" })).toContainText("目標 45分 まで");
});

test("a paint or an erase can be undone, and a tap the server refuses is taken back", async ({
  page,
  request,
  me,
}) => {
  const japanese = await createSubject(request, me, "国語");
  await createSubject(request, me, "理科");
  const { term } = await rpc<{ term: { id: string } }>(
    request,
    `${TIMETABLE}/CreateTerm`,
    {
      name: "やり直し",
      startDate: tokyoDate(-30),
      endDate: tokyoDate(90),
      layout: { preset: "LAYOUT_PRESET_HIGH_SCHOOL_6", weekdays: ALL_WEEKDAYS },
      observePublicHolidays: false,
    },
    me,
  );
  await rpc(
    request,
    `${TIMETABLE}/PutTimetableEntries`,
    {
      termId: term.id,
      entries: ALL_WEEKDAYS.map((weekday) => ({
        weekday,
        period: 1,
        subjectId: japanese,
        room: "3-B",
        memo: "漢字テスト",
      })),
    },
    me,
  );
  const saved = async () => {
    const { entries } = await rpc<{
      entries: {
        weekday: string;
        period: number;
        subject: { name: string };
        room?: string;
        memo?: string;
      }[];
    }>(request, `${TIMETABLE}/GetTerm`, { termId: term.id }, me);
    return entries;
  };

  await page.goto("/school/timetable");
  const grid = page.getByRole("table", { name: "時間割" });
  const first = `${tokyoWeekday()} 1限`;
  const second = `${tokyoWeekday()} 2限`;
  await page.getByRole("button", { name: "編集", exact: true }).click();

  // Painting over a class replaces it; undo brings the old class back with its room and memo.
  await page.getByRole("radio", { name: "理科" }).click();
  await grid.getByRole("button", { name: `${first} 国語` }).click();
  await expect(grid.getByRole("button", { name: `${first} 理科` })).toBeVisible();
  await expect(page.getByText("理科を入れたよ")).toBeVisible();
  await page.getByRole("button", { name: "元にもどす" }).click();
  await expect(grid.getByRole("button", { name: `${first} 国語` })).toBeVisible();
  await expect(page.getByText("元にもどしたよ")).toBeVisible();
  await expect(async () => {
    const entries = await saved();
    expect(entries).toHaveLength(7);
    expect(
      entries.every(
        (entry) =>
          entry.subject.name === "国語" && entry.room === "3-B" && entry.memo === "漢字テスト",
      ),
    ).toBe(true);
  }).toPass();

  // Erasing can be undone too.
  await page.getByRole("radio", { name: "消しゴム" }).click();
  await grid.getByRole("button", { name: `${first} 国語` }).click();
  await expect(grid.getByRole("button", { name: `${first} 空き` })).toBeVisible();
  await expect(page.getByText("コマを空にしたよ")).toBeVisible();
  await page.getByRole("button", { name: "元にもどす" }).click();
  await expect(grid.getByRole("button", { name: `${first} 国語` })).toBeVisible();
  await expect(async () => expect(await saved()).toHaveLength(7)).toPass();

  // A tap that does not reach the server leaves the grid as it was.
  await page.route("**/tokuly.school.v1.TimetableService/PutTimetableEntries", (route) =>
    route.abort(),
  );
  await page.getByRole("radio", { name: "理科" }).click();
  await grid.getByRole("button", { name: `${second} 空き` }).click();
  await expect(page.getByText("通信できなかったよ。接続を確認してね!")).toBeVisible();
  await expect(grid.getByRole("button", { name: `${second} 空き` })).toBeVisible();
  await grid.getByRole("button", { name: `${first} 国語` }).click();
  await expect(grid.getByRole("button", { name: `${first} 国語` })).toBeVisible();
  await expect(grid.getByRole("button", { name: `${first} 理科` })).toHaveCount(0);
  await page.unroute("**/tokuly.school.v1.TimetableService/PutTimetableEntries");
  expect(await saved()).toHaveLength(7);
});

test("a timer that could not be restored says so and offers to try again", async ({
  page,
  me: _me,
}) => {
  let failing = true;
  await page.route("**/tokuly.school.v1.StudyService/GetCurrentStudySession", (route) =>
    failing ? route.abort() : route.continue(),
  );
  await page.goto("/school/study");
  const timer = page.getByRole("region", { name: "勉強タイマー" });
  await expect(timer.getByText("タイマーのようすを読み込めなかったよ")).toBeVisible();

  failing = false;
  await timer.getByRole("button", { name: "もう一度" }).click();
  await expect(timer.getByRole("button", { name: "スタート!" })).toBeVisible();
});
