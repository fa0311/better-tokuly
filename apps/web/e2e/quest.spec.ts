import type { APIRequestContext, Locator, Page } from "@playwright/test";
import {
  type Account,
  adminAccount,
  befriend,
  createAccount,
  expect,
  rpc,
  signInAs,
  test,
} from "./support";

// Nothing is "done" through the quest API: XP, missions, badges and streaks
// follow what happens elsewhere in the app, a few seconds later (the server
// collects earnings before it writes and pushes them). So the tests do
// things through the other domains' APIs and wait for the result to show.

const QUEST = "tokuly.quest.v1";
const WALLET = "tokuly.wallet.v1.WalletService";
const CHAT = "tokuly.chat.v1";

/** How long the server may take to turn an event into XP, a badge or a streak. */
const SETTLE = 30_000;

interface MissionJson {
  id: string;
  key: string;
  state: string;
  title: { ja: string };
  reward?: { coins?: number; xp?: number };
}

interface MissionsJson {
  daily: MissionJson[];
  weekly?: MissionJson;
  rerollsLeft?: number;
}

const missionsOf = (request: APIRequestContext, as: Account) =>
  rpc<MissionsJson>(request, `${QUEST}.MissionService/ListMissions`, {}, as);

async function coinsOf(request: APIRequestContext, as: Account): Promise<number> {
  const { balance } = await rpc<{ balance?: { coins?: string } }>(
    request,
    `${WALLET}/GetBalance`,
    {},
    as,
  );
  return Number(balance?.coins ?? 0);
}

/** Sends `count` messages from `from` to `to` in their direct conversation. */
async function message(
  request: APIRequestContext,
  from: Account,
  to: Account,
  count = 1,
): Promise<void> {
  const { conversation } = await rpc<{ conversation: { id: string } }>(
    request,
    `${CHAT}.ConversationService/OpenDirect`,
    { userId: to.userId },
    from,
  );
  for (let index = 0; index < count; index++)
    await rpc(
      request,
      `${CHAT}.MessageService/SendMessage`,
      { conversationId: conversation.id, clientId: crypto.randomUUID(), text: `やっほー ${index}` },
      from,
    );
}

/** Resolves once the page's realtime connection is up, so that nothing pushed afterwards is missed. */
async function live(page: Page): Promise<void> {
  const connected = () => {
    const connection = (window as { __tokulyRealtime?: { connectionStatus: string } })
      .__tokulyRealtime;
    return connection ? connection.connectionStatus === "connected" : null;
  };
  if ((await page.evaluate(connected)) === null) await page.waitForTimeout(3000);
  else await page.waitForFunction(connected, null, { timeout: 20_000 });
}

/** The coins the balance pill in the header shows. */
async function pillCoins(page: Page): Promise<number> {
  const label = await page.getByRole("link", { name: /^コイン残高/ }).getAttribute("aria-label");
  return Number(label?.match(/コイン残高 ([\d,]+)/)?.[1]?.replaceAll(",", "") ?? Number.NaN);
}

const missionCard = (page: Page, title: string): Locator =>
  page.getByRole("article", { name: title, exact: true });

test("XP earned elsewhere arrives as a toast and moves the level bar", async ({
  page,
  request,
  me,
}) => {
  // The way in: the quest card on the hub.
  await page.goto("/me");
  await page.getByRole("link", { name: "クエスト", exact: true }).click();
  await expect(page).toHaveURL(/\/quest$/);

  // Signing up was worth 10 XP.
  const level = page.getByRole("region", { name: "いまのレベル" });
  await expect(level.getByTestId("quest-level")).toHaveText("1");
  await expect(level.getByTestId("quest-xp-into")).toHaveText("10 / 100 XP", { timeout: SETTLE });
  await expect(level.getByRole("progressbar", { name: "つぎのレベルまで" })).toHaveAttribute(
    "aria-valuenow",
    "10",
  );

  // The login bonus is claimed somewhere else (here: through the API) and is worth 10 XP.
  await live(page);
  await rpc(request, `${WALLET}/ClaimDailyBonus`, {}, me);
  // In the toast: the missions drawn for the account may show "+10 XP" as their reward.
  await expect(
    page.locator("[data-sonner-toast]").getByText("+10 XP", { exact: true }),
  ).toBeVisible({ timeout: SETTLE });
  await expect(level.getByTestId("quest-xp-into")).toHaveText("20 / 100 XP");
  await expect(level.getByRole("progressbar", { name: "つぎのレベルまで" })).toHaveAttribute(
    "aria-valuenow",
    "20",
  );

  // Today's XP by source: the "showing up" source is full, with its daily cap.
  const sources = page.getByRole("region", { name: "きょうのXP" });
  const login = sources.getByRole("listitem").filter({ hasText: "ログイン" });
  await expect(login).toContainText("20 / 20");
  await expect(login).toContainText("MAX");

  // Claiming again gives nothing: a reward is granted once.
  await rpc(request, `${WALLET}/ClaimDailyBonus`, {}, me).catch(() => {});
  await page.waitForTimeout(5_000);
  await expect(level.getByTestId("quest-xp-into")).toHaveText("20 / 100 XP");

  // How XP is earned, with the caps, and the day's record.
  await sources.getByRole("link", { name: "XPのもらいかた" }).click();
  await expect(page.getByRole("region", { name: "チャット" })).toContainText("メッセージを送る");
  await expect(page.getByRole("region", { name: "ログイン" })).toContainText("きょう 20 / 20 XP");
  await page.getByRole("tab", { name: "きろく" }).click();
  await expect(page.getByRole("listitem").filter({ hasText: "きょう" })).toContainText("+20 XP");
});

test("a finished mission is claimed and the coins arrive", async ({ page, request }) => {
  test.slow();
  // The day's missions are drawn per account. These can be finished from here.
  const doable = ["add_friend", "xp_150", "xp_50", "daily_bonus", "messages_5", "messages_20"];
  let me: Account | undefined;
  for (let attempt = 0; attempt < 8 && !me; attempt++) {
    const account = await createAccount(request, "quester");
    const drawn = await missionsOf(request, account);
    if (drawn.daily.some((mission) => doable.includes(mission.key))) me = account;
  }
  if (!me) throw new Error("no account was offered a mission this test can finish");
  await signInAs(page, me);

  await page.goto("/quest");
  await expect(page.getByRole("region", { name: "きょうのミッション" })).toBeVisible({
    timeout: SETTLE,
  });
  await live(page);

  // Five new friends (100 XP, and "add a friend"), the login bonus, and a chat (40 XP): over 150 XP.
  const friends: Account[] = [];
  for (let index = 0; index < 5; index++) {
    const friend = await createAccount(request, `pal${index}`);
    await befriend(request, me, friend);
    friends.push(friend);
  }
  await rpc(request, `${WALLET}/ClaimDailyBonus`, {}, me);
  await message(request, me, friends[0] as Account, 20);

  let done: MissionJson | undefined;
  await expect(async () => {
    done = (await missionsOf(request, me as Account)).daily.find(
      (mission) => mission.state === "MISSION_STATE_COMPLETED",
    );
    expect(done, "a daily mission is completed").toBeTruthy();
  }).toPass({ timeout: 60_000 });
  const mission = done as MissionJson;

  // The page heard about it: the card offers the reward.
  const card = missionCard(page, mission.title.ja);
  await expect(card).toHaveAttribute("data-state", "completed", { timeout: SETTLE });

  // Level-ups pay coins too: wait until the balance is quiet, then claim.
  let before = -1;
  await expect(async () => {
    const coins = await coinsOf(request, me as Account);
    const quiet = coins === before;
    before = coins;
    expect(quiet).toBe(true);
    expect(await pillCoins(page)).toBe(coins);
  }).toPass({ timeout: 60_000, intervals: [4_000] });

  await card.getByRole("button", { name: "受け取る" }).click();
  await expect(card).toHaveAttribute("data-state", "claimed");
  await expect(card.getByText("受け取りずみ")).toBeVisible();
  // The claim is celebrated, without taking the page away.
  // (It waits its turn behind the badges and the level-up the new friends brought.)
  await expect(page.getByTestId("quest-reveal").filter({ hasText: mission.title.ja })).toBeVisible({
    timeout: SETTLE,
  });

  const reward = mission.reward?.coins ?? 0;
  expect(reward).toBeGreaterThan(0);
  await expect
    .poll(() => pillCoins(page), { timeout: SETTLE })
    .toBeGreaterThanOrEqual(before + reward);
  expect(await coinsOf(request, me)).toBeGreaterThanOrEqual(before + reward);
  await expect(page.getByText("受け取り 1 / 3")).toBeVisible();

  // Asking for the reward again gives nothing more.
  const again = await rpc<{ granted?: boolean }>(
    request,
    `${QUEST}.MissionService/ClaimMission`,
    { missionId: mission.id },
    me,
  );
  expect(again.granted ?? false).toBe(false);
});

test("one mission a day can be swapped for another", async ({ page, request, me }) => {
  // Missions are only drawn from what people on this server do: make sure something is going on.
  const friend = await createAccount(request, "swapper");
  await befriend(request, me, friend);
  await message(request, friend, me, 2);
  await rpc(request, `${WALLET}/ClaimDailyBonus`, {}, friend);
  await expect
    .poll(
      async () => {
        const { progress } = await rpc<{ progress?: { xpToday?: number } }>(
          request,
          `${QUEST}.QuestService/GetProgress`,
          {},
          friend,
        );
        return progress?.xpToday ?? 0;
      },
      { timeout: SETTLE },
    )
    .toBeGreaterThanOrEqual(44);

  const before = await missionsOf(request, me);
  const first = before.daily.find(
    (mission) => mission.state === "MISSION_STATE_ACTIVE",
  ) as MissionJson;

  await page.goto("/quest");
  await expect(page.getByText("ミッションの入れ替え: きょうはあと1回できるよ")).toBeVisible({
    timeout: SETTLE,
  });
  await missionCard(page, first.title.ja)
    .getByRole("button", { name: `「${first.title.ja}」を入れ替える` })
    .click();
  const dialog = page.getByRole("dialog");
  await expect(dialog).toContainText("入れ替えは1日1回だけ!");
  await dialog.getByRole("button", { name: "入れ替える" }).click();
  await expect(page.getByText("ミッションを入れ替えたよ!")).toBeVisible();

  // Another mission took the slot, and the day's swap is used: no card offers one any more.
  const after = await missionsOf(request, me);
  const swapped = after.daily.find((mission) => mission.id === first.id) as MissionJson;
  expect(swapped.key).not.toBe(first.key);
  await expect(missionCard(page, swapped.title.ja)).toBeVisible();
  await expect(missionCard(page, first.title.ja)).toHaveCount(0);
  await expect(page.getByText("ミッションの入れ替え: きょうはもう使ったよ")).toBeVisible();
  await expect(page.getByRole("button", { name: /を入れ替える$/ })).toHaveCount(0);
});

test("a badge unlocks, is celebrated and goes on the profile showcase", async ({
  page,
  request,
  browser,
  baseURL,
  me,
}) => {
  test.slow();
  await page.goto("/quest/badges");
  // The badge every account starts with; secret ones stay secret.
  await expect(page.getByTestId("quest-badge-count")).toHaveText(/^1 \/ \d+ こ$/, {
    timeout: SETTLE,
  });
  await expect(page.getByRole("link", { name: "ようこそ!(ゲットずみ)" })).toBeVisible();
  await expect(page.getByRole("link", { name: "はじめての友達(まだ)" })).toBeVisible();
  const secrets = page.getByRole("region", { name: "シークレット" });
  await expect(secrets.getByText("ゲットするまでひみつ!").first()).toBeVisible();

  // Making a first friend (in another domain) earns one.
  await live(page);
  const friend = await createAccount(request, "buddy");
  await befriend(request, me, friend);
  const reveal = page.getByTestId("quest-reveal").filter({ hasText: "はじめての友達" });
  await expect(reveal).toBeVisible({ timeout: SETTLE });
  await expect(reveal).toContainText("バッジ ゲット!");
  await reveal.getByRole("button", { name: "見る" }).click();

  // The badge's page: earned, and it can go on the profile.
  await expect(page).toHaveURL(/\/quest\/badges\/first_friend$/);
  await expect(page.getByTestId("quest-badge-name")).toHaveText("はじめての友達");
  await expect(page.getByText(/にゲット!$/)).toBeVisible();
  await page.getByRole("button", { name: "プロフィールにかざる" }).click();
  await expect(page.getByText("プロフィールにかざったよ!")).toBeVisible();
  await expect(page.getByRole("button", { name: "プロフィールからはずす" })).toBeVisible();

  // The showcase editor: add a second one and put it first.
  await page.goto("/quest/showcase");
  const shown = page.getByRole("region", { name: "プロフィールにかざるバッジ" });
  await expect(shown).toContainText("1 / 5");
  const owned = page.getByRole("region", { name: "持ってるバッジ" });
  await expect(owned.getByRole("button", { name: "はじめての友達" })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  await owned.getByRole("button", { name: "ようこそ!" }).click();
  await expect(shown).toContainText("2 / 5");
  await shown.getByRole("button", { name: "ようこそ!をならべかえる" }).press("ArrowUp");
  await expect
    .poll(async () => {
      const { showcase } = await rpc<{ showcase?: { key: string }[] }>(
        request,
        `${QUEST}.BadgeService/ListUserBadges`,
        {},
        me,
      );
      return showcase?.map((badge) => badge.key);
    })
    .toEqual(["welcome", "first_friend"]);

  // On the profile, in that order; the friend sees it too.
  await page.goto(`/u/${me.handle}`);
  const showcase = page.getByRole("region", { name: "バッジ", exact: true });
  await expect(showcase.getByRole("link")).toHaveText([/かざる/, /ようこそ!/, /はじめての友達/]);

  const viewer = await browser.newContext({
    baseURL,
    locale: "ja-JP",
    viewport: page.viewportSize(),
    hasTouch: (page.viewportSize()?.width ?? 1000) < 600,
  });
  const theirs = await viewer.newPage();
  await signInAs(theirs, friend);
  await theirs.goto(`/u/${me.handle}`);
  const seen = theirs.getByRole("region", { name: "バッジ", exact: true });
  await expect(seen.getByRole("link", { name: "はじめての友達" })).toBeVisible({ timeout: SETTLE });
  // …and on the person's quest page, with their level.
  await seen.getByRole("link", { name: "もっと見る" }).click();
  await expect(theirs).toHaveURL(new RegExp(`/quest/u/${me.handle}$`));
  await expect(
    theirs.getByRole("region", { name: "かざってるバッジ" }).getByRole("link"),
  ).toHaveText([/ようこそ!/, /はじめての友達/]);
  // Back returns to where the person page was opened from.
  await theirs.getByRole("button", { name: "back" }).click();
  await expect(theirs).toHaveURL(new RegExp(`/u/${me.handle}$`));
  await viewer.close();
});

test("a title is worn and shows on the profile", async ({ page, request, me }) => {
  test.slow();
  await page.goto("/quest/showcase");
  // Nothing to wear yet.
  await expect(page.getByText("称号つきのバッジをゲットすると、ここでえらべるよ")).toBeVisible();
  await live(page);

  // Staff hand out a badge that comes with a title.
  const admin = await adminAccount(request);
  const awarded = await rpc<{ awarded?: boolean }>(
    request,
    `${QUEST}.QuestAdminService/AwardBadge`,
    { userId: me.userId, badgeKey: "bug_hunter", reason: "e2e: found a bug" },
    admin,
  );
  expect(awarded.awarded).toBe(true);

  const titles = page.getByRole("radiogroup", { name: "称号" });
  const option = titles.getByRole("radio", { name: "バグハンター" });
  await expect(option).toBeVisible({ timeout: SETTLE });
  await expect(titles.getByRole("radio", { name: "つけない" })).toBeChecked();
  await option.click();
  await expect(page.getByText("称号をつけたよ!")).toBeVisible();
  await expect(option).toBeChecked();

  // Next to the level on the quest home and on the profile.
  await page.goto("/quest");
  await expect(page.getByRole("region", { name: "いまのレベル" })).toContainText("バグハンター");
  await page.goto(`/u/${me.handle}`);
  const status = page.getByTestId("quest-profile-status");
  await expect(status).toContainText("Lv.1");
  await expect(status).toContainText("バグハンター");

  // Taking the badge back takes the title with it.
  await rpc(
    request,
    `${QUEST}.QuestAdminService/RevokeBadge`,
    { userId: me.userId, badgeKey: "bug_hunter", reason: "e2e: tidy up" },
    admin,
  );
  await page.reload();
  await expect(page.getByTestId("quest-profile-status")).toContainText("Lv.1");
  await expect(page.getByTestId("quest-profile-status")).not.toContainText("バグハンター");
});

test("a streak starts when two friends write to each other, and the friend is on the leaderboard", async ({
  page,
  request,
  me,
}) => {
  test.slow();
  const friend = await createAccount(request, "streaker");
  await befriend(request, me, friend);

  await page.goto("/quest/streaks");
  await expect(page.getByText("まだストリークがないよ")).toBeVisible();
  await live(page);

  // One of the two is not a streak yet; both are.
  await message(request, me, friend);
  await message(request, friend, me);
  const row = page.getByRole("listitem", { name: `${friend.displayName}と1日連続` });
  await expect(row).toBeVisible({ timeout: SETTLE });
  await expect(row).toContainText("きょうのぶんはOK!");
  await expect(row).toContainText("あと2日で3日!");
  await expect(row).toContainText("さいこう記録 1日");

  // On the friend's profile, for the two of them.
  await page.goto(`/u/${friend.handle}`);
  await expect(page.getByTestId("quest-profile-status")).toContainText("1日連続");

  // The ranking of the week lists both; the streak board has the day.
  await page.goto("/quest/ranking");
  const boards = page.getByRole("region", { name: "ともだちランキング" });
  const xp = boards.getByRole("list", { name: "XP" });
  await expect(xp.getByRole("listitem")).toHaveCount(2);
  await expect(xp.getByRole("link", { name: new RegExp(friend.displayName) })).toBeVisible();
  await expect(xp.getByRole("listitem").filter({ hasText: "じぶん" })).toContainText("XP");
  await boards.getByRole("tab", { name: /ストリーク/ }).click();
  const streaks = boards.getByRole("list", { name: "ストリーク" });
  await expect(streaks.getByRole("link", { name: new RegExp(friend.displayName) })).toContainText(
    "1日",
  );

  // The season of the month, with its badge and what it takes.
  const season = page.getByRole("region", { name: "今月のシーズン" });
  await expect(season).toContainText("シーズン");
  await expect(season).toContainText("1位まで");
  await expect(season.getByText(/^あと\d+(日|時間|分)$/)).toBeVisible();

  // Leaving the rankings: the friend no longer finds this account there.
  await page.goto("/quest/settings");
  await page.getByRole("switch", { name: "ランキングに出る" }).click();
  await expect
    .poll(
      async () => {
        const board = await rpc<{ entries?: { user?: { id: string } }[] }>(
          request,
          `${QUEST}.LeaderboardService/GetLeaderboard`,
          { board: "xp", period: "LEADERBOARD_PERIOD_WEEK" },
          friend,
        );
        return board.entries?.some((entry) => entry.user?.id === me.userId) ?? false;
      },
      { timeout: SETTLE },
    )
    .toBe(false);
});

test("with reduced motion a badge is a toast, not a reveal", async ({ page, request, me }) => {
  test.slow();
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.goto("/quest");
  await expect(page.getByRole("region", { name: "いまのレベル" })).toBeVisible({ timeout: SETTLE });
  await live(page);

  const admin = await adminAccount(request);
  await rpc(
    request,
    `${QUEST}.QuestAdminService/AwardBadge`,
    { userId: me.userId, badgeKey: "event_winner", reason: "e2e: reduced motion" },
    admin,
  );
  // Matched by its text: a notification toast may announce the badge as well.
  await expect(page.getByText("🎉 バッジ「イベント入賞」をゲット!")).toBeVisible({
    timeout: SETTLE,
  });
  await expect(page.getByTestId("quest-reveal")).toHaveCount(0);
});

test("someone who is not a friend sees the showcase, but not every badge or the streak", async ({
  page,
  request,
  browser,
  baseURL,
  me,
}) => {
  test.slow();
  // `owner` has the welcome and first-friend badges, shows none, and has a streak with `me`.
  // What they do here may earn more on its own (the early bird, when the test runs between 5
  // and 7 in the morning in Tokyo), so the count the stranger sees is read from the owner.
  const owner = await createAccount(request, "owner");
  await befriend(request, me, owner);
  await message(request, me, owner);
  await message(request, owner, me);
  await expect
    .poll(
      async () => {
        const { streak } = await rpc<{ streak?: { days?: number } }>(
          request,
          `${QUEST}.StreakService/GetStreak`,
          { userId: owner.userId },
          me,
        );
        return streak?.days ?? 0;
      },
      { timeout: SETTLE },
    )
    .toBe(1);
  // The first-friend badge is awarded a moment after the friendship, like the streak after the
  // messages: the quest page shows what was earned when it opened.
  const badgesOf = async (viewer: Account) => {
    const { badges } = await rpc<{ badges?: { key: string }[] }>(
      request,
      `${QUEST}.BadgeService/ListUserBadges`,
      { userId: owner.userId },
      viewer,
    );
    return badges?.map((badge) => badge.key) ?? [];
  };
  await expect.poll(() => badgesOf(me), { timeout: SETTLE }).toContain("first_friend");
  const earned = await badgesOf(owner);
  expect(earned).toEqual(expect.arrayContaining(["welcome", "first_friend"]));

  // The friend: the streak on the profile, and every badge on the quest page.
  await page.goto(`/u/${owner.handle}`);
  await expect(page.getByTestId("quest-profile-status")).toContainText("1日連続", {
    timeout: SETTLE,
  });
  await page.goto(`/quest/u/${owner.handle}`);
  await expect(page.getByRole("region", { name: "ほかのバッジ" })).toContainText("はじめての友達", {
    timeout: SETTLE,
  });

  // A stranger: the level, but neither the streak nor the badges that are only for friends.
  const stranger = await createAccount(request, "stranger");
  const context = await browser.newContext({
    baseURL,
    locale: "ja-JP",
    viewport: page.viewportSize(),
    hasTouch: (page.viewportSize()?.width ?? 1000) < 600,
  });
  const theirs = await context.newPage();
  await signInAs(theirs, stranger);
  await theirs.goto(`/u/${owner.handle}`);
  const status = theirs.getByTestId("quest-profile-status");
  await expect(status).toContainText("Lv.1", { timeout: SETTLE });
  await expect(status).not.toContainText("日連続");
  await theirs.goto(`/quest/u/${owner.handle}`);
  await expect(theirs.getByRole("region", { name: "かざってるバッジ" })).toContainText(
    "まだ何もかざってないみたい",
    { timeout: SETTLE },
  );
  // How many badges the owner has, but not which: neither on the page nor from the API.
  await expect(theirs.getByText(`バッジ ${earned.length}こ`)).toBeVisible();
  await expect(theirs.getByRole("region", { name: "ほかのバッジ" })).toHaveCount(0);
  expect(await badgesOf(stranger)).toEqual([]);
  await expect(theirs.getByRole("region", { name: "ふたりのストリーク" })).toHaveCount(0);
  await context.close();
});
