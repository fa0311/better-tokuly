import type { Browser, Page, TestInfo } from "@playwright/test";
import { type Account, befriend, createAccount, expect, rpc, signInAs, test } from "./support";

const FREE = "tokuly.plans.v1.FreeService";
const PLAN = "tokuly.plans.v1.PlanService";
const POLL = "tokuly.plans.v1.PollService";
const ANNIVERSARY = "tokuly.plans.v1.AnniversaryService";

/** A second person's browser, signed in, at the size of the running project. */
async function browserOf(browser: Browser, testInfo: TestInfo, account: Account): Promise<Page> {
  const context = await browser.newContext({ baseURL: testInfo.project.use.baseURL, locale: "ja-JP", timezoneId: "Asia/Tokyo", viewport: testInfo.project.use.viewport, hasTouch: testInfo.project.use.hasTouch });
  const page = await context.newPage();
  await signInAs(page, account);
  return page;
}

/** `YYYY-MM-DD` of the day `offset` days from now, in Tokyo (where the browsers of these tests are). */
function tokyoDay(offset = 0): string {
  return new Intl.DateTimeFormat("sv-SE", { timeZone: "Asia/Tokyo" }).format(new Date(Date.now() + offset * 86_400_000));
}

test("ひま: two taps set it, a friend sees it live and answers in one tap", async ({ page, request, me, browser }, testInfo) => {
  const friend = await createAccount(request, "Aoi");
  await befriend(request, me, friend);
  const friendPage = await browserOf(browser, testInfo, friend);
  await friendPage.goto("/plans");
  await expect(friendPage.getByText("今はみんな忙しいみたい")).toBeVisible();

  await page.goto("/plans");
  await page.getByRole("button", { name: /^ひま！/ }).click();
  await page.getByRole("button", { name: /^今ひま/ }).click();
  await expect(page.getByText("ひまを公開したよ！")).toBeVisible();
  // It says that it ends by itself.
  await page.goto("/plans/free");
  await expect(page.getByTestId("my-free-status")).toContainText("ひま中！");
  await expect(page.getByTestId("my-free-status")).toContainText(/あと.+で自動で消えるよ/);

  // The friend's strip shows it without a reload.
  const tile = friendPage.getByTestId("free-friend").filter({ hasText: me.displayName });
  await expect(tile).toBeVisible();
  await tile.getByRole("button", { name: "あそぼ！" }).click();
  // What follows the answer: a sheet offering a chat or a plan for the two.
  const sheet = friendPage.getByRole("dialog");
  await expect(sheet.getByText("「あそぼ！」を送ったよ")).toBeVisible();
  await expect(sheet.getByRole("button", { name: "予定をたてる" })).toBeVisible();
  await sheet.getByRole("button", { name: "close" }).click();
  await expect(tile.getByRole("button", { name: "送った" })).toBeVisible();

  // The answer arrives live on the owner's page.
  await expect(page.getByText("「あそぼ！」が来てるよ")).toBeVisible();
  await expect(page.getByRole("main").getByText(friend.displayName).first()).toBeVisible();

  // Ending it early takes it off the friend's strip.
  await page.getByRole("button", { name: "おわりにする" }).click();
  await expect(page.getByText("今、ひま？")).toBeVisible();
  await expect(friendPage.getByTestId("free-friend")).toHaveCount(0);
  await friendPage.context().close();
});

test("a plan: created with a friend invited, the friend answers, the checklist and the cost split add up", async ({ page, request, me, browser }, testInfo) => {
  const friend = await createAccount(request, "Haruto");
  await befriend(request, me, friend);

  await page.goto("/plans/new");
  await page.getByLabel("なにする？").fill("放課後カラオケ");
  await page.getByRole("button", { name: "🎤" }).click();
  await page.getByRole("radio", { name: "日にちだけ" }).click();
  await page.getByLabel("日にち").fill(tokyoDay(3));
  await page.getByLabel("どこで？").fill("カラオケ館");
  await page.getByRole("button", { name: "ともだちをさそう" }).click();
  await page.getByRole("checkbox", { name: new RegExp(friend.displayName) }).click();
  await page.getByRole("button", { name: /これでOK/ }).click();
  await page.getByRole("button", { name: "この予定でさそう！" }).click();

  await expect(page).toHaveURL(/\/plans\/[0-9a-f-]{36}$/);
  await expect(page.getByRole("heading", { name: "放課後カラオケ", level: 2 })).toBeVisible();
  await expect(page.getByText("カラオケ館")).toBeVisible();
  await expect(page.getByText("まだ返事なし · 1")).toBeVisible();
  const planId = page.url().split("/").pop() as string;

  // The friend finds the invitation and answers; the host sees it without a reload.
  const friendPage = await browserOf(browser, testInfo, friend);
  await friendPage.goto("/plans");
  await friendPage.getByRole("link", { name: /放課後カラオケ/ }).click();
  await friendPage.getByRole("radio", { name: /行く！/ }).click();
  await expect(friendPage.getByText("「行く！」で返事したよ")).toBeVisible();
  await expect(page.getByText("行く · 2")).toBeVisible();

  // 持ち物: the host adds, the friend takes it, the host sees who brings it.
  await page.getByLabel(/持ち物を追加/).fill("おかし");
  await page.getByRole("button", { name: "追加", exact: true }).click();
  const item = friendPage.getByTestId("checklist-item").filter({ hasText: "おかし" });
  await item.getByRole("button", { name: "持ってく" }).click();
  await expect(page.getByTestId("checklist-item").filter({ hasText: "おかし" })).toContainText(friend.displayName);
  await page.getByRole("checkbox", { name: "おかしを完了にする" }).click();
  await expect(item.getByRole("checkbox")).toBeChecked();

  // 割り勘: 3,000 yen paid by the host, split between the two.
  await page.getByRole("button", { name: "支払いを追加" }).click();
  await page.getByLabel("なに代？").fill("カラオケ代");
  await page.getByLabel(/^金額/).fill("3000");
  await page.getByRole("button", { name: "追加する" }).click();
  await expect(page.getByTestId("my-balance")).toHaveText("あと1,500円 もらえるよ");
  await expect(page.getByTestId("split-summary")).toContainText("3,000円");
  await expect(friendPage.getByTestId("my-balance")).toHaveText("あと1,500円 払ってね");
  // The one who owes cannot mark it paid; the one who received the money does.
  await expect(friendPage.getByRole("button", { name: "受け取った！" })).toHaveCount(0);
  await page.getByTestId("transfer").getByRole("button", { name: "受け取った！" }).click();
  await page.getByRole("dialog").getByRole("button", { name: "受け取った！" }).click();
  await expect(page.getByTestId("my-balance")).toHaveText("あなたは精算ずみ");
  await expect(friendPage.getByTestId("my-balance")).toHaveText("あなたは精算ずみ");
  const split = await rpc<{ settlements: { amount: string }[] }>(request, `${PLAN}/GetSplit`, { planId }, me);
  expect(split.settlements.map((settlement) => settlement.amount)).toEqual(["1500"]);
  await friendPage.context().close();
});

test("a date poll: three days, two people answer, the best one is highlighted and becomes a plan", async ({ page, request, me }) => {
  const friend = await createAccount(request, "Yui");
  await befriend(request, me, friend);
  const days = [tokyoDay(5), tokyoDay(6), tokyoDay(7)];
  const { poll } = await rpc<{ poll: { id: string; shortId: string; slots: { id: string }[] } }>(
    request,
    `${POLL}/CreatePoll`,
    { title: "打ち上げいつにする？", slots: days.map((date) => ({ date })), inviteeIds: [friend.userId], timeZone: "Asia/Tokyo" },
    me,
  );
  const [first, second, third] = poll.slots.map((slot) => slot.id);
  await rpc(
    request,
    `${POLL}/AnswerPoll`,
    {
      pollId: poll.id,
      answers: [
        { slotId: first, answer: "POLL_ANSWER_NO" },
        { slotId: second, answer: "POLL_ANSWER_YES" },
        { slotId: third, answer: "POLL_ANSWER_MAYBE" },
      ],
    },
    friend,
  );

  await page.goto(`/plans/polls/${poll.shortId}`);
  const rows = page.getByTestId("poll-slot");
  await expect(rows).toHaveCount(3);
  await rows.nth(0).getByRole("radio", { name: /△/ }).click();
  await rows.nth(1).getByRole("radio", { name: /○/ }).click();
  await rows.nth(2).getByRole("radio", { name: /×/ }).click();
  await page.getByRole("button", { name: "回答する" }).click();
  await expect(page.getByText("回答したよ！")).toBeVisible();

  // The second day has two ○: it is the suggestion, in the summary, the rows and the table.
  await expect(page.getByTestId("poll-answered")).toHaveText("2/2人が回答");
  await expect(page.getByTestId("poll-best")).toContainText("○2 △0 ×0");
  await expect(rows.nth(1)).toHaveAttribute("data-best", "true");
  await expect(rows.nth(0)).not.toHaveAttribute("data-best", "true");
  await expect(page.getByTestId("poll-grid").locator("tbody tr").nth(1)).toHaveAttribute("data-best", "true");
  await expect(page.getByTestId("poll-grid").getByRole("columnheader", { name: friend.displayName })).toBeVisible();

  await page.getByRole("button", { name: "この日程で決める" }).click();
  await page.getByRole("button", { name: "この日で決定！" }).click();
  await expect(page).toHaveURL(/\/plans\/[0-9a-f-]{36}$/);
  await expect(page.getByRole("heading", { name: "打ち上げいつにする？", level: 2 })).toBeVisible();
  await expect(page.getByText("日程調整から")).toBeVisible();
  // The friend said ○ for that day, so they are invited to the plan.
  await expect(page.getByText("まだ返事なし · 1")).toBeVisible();
  const planId = page.url().split("/").pop() as string;
  const { plan } = await rpc<{ plan: { time: { startDate: string } } }>(request, `${PLAN}/GetPlan`, { planId }, me);
  expect(plan.time.startDate).toBe(days[1]);
});

test("a countdown is created, pinned to the profile, and friends see it there", async ({ page, request, me, browser }, testInfo) => {
  const friend = await createAccount(request, "Sota");
  await befriend(request, me, friend);

  await page.goto("/plans?tab=moments");
  await page.getByRole("button", { name: "つくる", exact: true }).click();
  await page.getByRole("button", { name: /テスト/ }).click();
  await page.getByLabel("なんの日？").fill("期末テスト");
  await page.getByLabel("その日").fill(tokyoDay(12));
  await page.getByRole("button", { name: "つくる！" }).click();
  const card = page.getByTestId("countdown-card").filter({ hasText: "期末テスト" });
  await expect(card).toContainText("12");

  await card.click();
  await expect(page.getByTestId("countdown-hero")).toContainText("あと");
  await page.getByRole("button", { name: "プロフィールにピン留め" }).click();
  await expect(page.getByText("プロフィールにピン留めしたよ！")).toBeVisible();

  await page.goto(`/u/${me.handle}`);
  await expect(page.getByTestId("profile-countdown")).toContainText("期末テスト");
  await expect(page.getByTestId("profile-countdown")).toContainText("12");

  // A friend sees the pin on the profile and can open it.
  const friendPage = await browserOf(browser, testInfo, friend);
  await friendPage.goto(`/u/${me.handle}`);
  await friendPage.getByTestId("profile-countdown").click();
  await expect(friendPage.getByTestId("countdown-hero")).toContainText("期末テスト");
  await friendPage.context().close();
});

test("on its day a countdown takes over the screen", async ({ page, request, me }) => {
  const { countdown } = await rpc<{ countdown: { id: string } }>(request, `${ANNIVERSARY}/CreateCountdown`, { title: "文化祭", emoji: "🏮", date: tokyoDay(0) }, me);
  await page.goto(`/plans/countdowns/${countdown.id}`);
  const moment = page.getByTestId("today-moment");
  await expect(moment).toContainText("今日がその日！");
  await expect(moment).toContainText("文化祭");
  await moment.getByRole("button", { name: "やったー！" }).click();
  await expect(moment).toHaveCount(0);
  await expect(page.getByTestId("countdown-hero")).toContainText("今日！");
});

test("the calendar shows plans, poll deadlines, countdowns and friends' birthdays, and exports the month", async ({ page, request, me }) => {
  const friend = await createAccount(request, "Mio");
  await befriend(request, me, friend);
  const today = tokyoDay(0);
  await rpc(request, `${PLAN}/CreatePlan`, { title: "みんなでごはん", emoji: "🍜", time: { startDate: today, timeZone: "Asia/Tokyo" }, inviteeIds: [friend.userId] }, me);
  await rpc(request, `${ANNIVERSARY}/CreateCountdown`, { title: "ライブ", emoji: "🎤", date: today }, me);
  const deadline = new Date(Date.now() + 2 * 86_400_000);
  await rpc(request, `${POLL}/CreatePoll`, { title: "旅行の日程", slots: [{ date: tokyoDay(20) }], deadline: deadline.toISOString(), timeZone: "Asia/Tokyo" }, me);
  await rpc(request, `${ANNIVERSARY}/UpdatePlansSettings`, { shareBirthday: true }, friend);

  await page.goto("/plans/calendar");
  const day = page.getByTestId("calendar-day");
  await expect(day.getByTestId("calendar-entry").filter({ hasText: "みんなでごはん" })).toContainText("予定");
  await expect(day.getByTestId("calendar-entry").filter({ hasText: "ライブ" })).toContainText("カウントダウン");
  // The month page marks the day.
  await expect(page.locator('button[aria-pressed="true"]').getByTestId("calendar-marks")).toBeVisible();

  // The whole month goes out as a calendar file.
  const [download] = await Promise.all([page.waitForEvent("download"), page.getByRole("button", { name: /\.ics/ }).click()]);
  expect(download.suggestedFilename()).toBe(`tokuly-${today.slice(0, 7)}.ics`);
  const ics = await (await import("node:fs/promises")).readFile(await download.path(), "utf8");
  expect(ics).toContain("SUMMARY:🍜 みんなでごはん");
  expect(ics).toContain(`DTSTART;VALUE=DATE:${today.replaceAll("-", "")}`);

  await page.goto(`/plans/calendar?day=${tokyoDay(2)}`);
  await expect(day.getByTestId("calendar-entry").filter({ hasText: "旅行の日程" })).toBeVisible();

  // Test accounts are born on April 2: the friend who shares it shows up on the next one.
  const year = Number(today.slice(0, 4)) + (today.slice(5) > "04-02" ? 1 : 0);
  await page.goto(`/plans/calendar?day=${year}-04-02`);
  await expect(day.getByTestId("calendar-entry").filter({ hasText: friend.displayName })).toBeVisible();

  // An entry leads to what it stands for.
  await page.goto("/plans/calendar");
  await day.getByTestId("calendar-entry").filter({ hasText: "みんなでごはん" }).click();
  await expect(page.getByRole("heading", { name: "みんなでごはん", level: 2 })).toBeVisible();
});

test("the hub is reachable from じぶん and lists what needs an answer", async ({ page, request, me }) => {
  const friend = await createAccount(request, "Rin");
  await befriend(request, me, friend);
  await rpc(request, `${PLAN}/CreatePlan`, { title: "プリ撮りにいこ", inviteeIds: [me.userId] }, friend);
  await rpc(request, `${FREE}/SetFreeStatus`, { window: "FREE_WINDOW_TODAY", note: "だれかあそぼ", audience: "FREE_AUDIENCE_FRIENDS" }, friend);

  await page.goto("/me");
  await page.getByRole("main").getByRole("link", { name: "あそぶ" }).click();
  await expect(page).toHaveURL(/\/plans$/);
  await expect(page.getByTestId("free-friend").filter({ hasText: friend.displayName })).toContainText("だれかあそぼ");

  await page.getByRole("radio", { name: /未回答/ }).click();
  await page.getByRole("link", { name: /プリ撮りにいこ/ }).click();
  await expect(page.getByText("行く？ 返事してね！")).toBeVisible();
  await page.getByRole("radio", { name: /行けない/ }).click();
  await expect(page.getByText("「行けない」で返事したよ")).toBeVisible();

  // Leaving is for good, and says so first.
  await page.getByRole("button", { name: "予定のメニュー" }).click();
  await page.getByRole("menuitem", { name: "予定から抜ける" }).click();
  await expect(page.getByText("もうさそってもらえなくなるよ")).toBeVisible();
  await page.getByRole("button", { name: "抜ける", exact: true }).click();
  await expect(page).toHaveURL(/\/plans$/);
  const again = await rpc<{ invitedCount: number }>(request, `${PLAN}/InviteToPlan`, { planId: (await rpc<{ plans: { id: string }[] }>(request, `${PLAN}/ListPlans`, {}, friend)).plans[0]?.id, userIds: [me.userId] }, friend);
  expect(again.invitedCount ?? 0).toBe(0);
});
