import type { APIRequestContext, Browser, Page, TestInfo } from "@playwright/test";
import { type Account, befriend, createAccount, expect, rpc, signInAs, test } from "./support";

// Rooms and hangouts carry sound over LiveKit. Chrome's fake microphone (a beep) stands in for a
// real one and no permission prompt appears. Launch options replace the ones of the configuration
// as a whole, so its flags are repeated here.
test.use({
  launchOptions: {
    args: [
      "--use-angle=swiftshader",
      "--enable-unsafe-swiftshader",
      "--ignore-gpu-blocklist",
      "--enable-webgl",
      "--disable-dev-shm-usage",
      "--use-fake-device-for-media-stream",
      "--use-fake-ui-for-media-stream",
      // Rooms play as soon as someone enters, also in a browser nobody has tapped in yet.
      "--autoplay-policy=no-user-gesture-required",
    ],
  },
});

// Two browsers enter the same LiveKit room in most of these tests.
test.setTimeout(90_000);

const SPACES = "tokuly.spaces.v1.SpaceService";
const HANGOUTS = "tokuly.spaces.v1.HangoutService";

interface Space {
  id: string;
  title: string;
  state: string;
}

/** A second person's browser, signed in, at the size of the running project. */
async function browserOf(browser: Browser, testInfo: TestInfo, account: Account): Promise<Page> {
  const context = await browser.newContext({
    baseURL: testInfo.project.use.baseURL,
    locale: "ja-JP",
    timezoneId: testInfo.project.use.timezoneId,
    viewport: testInfo.project.use.viewport,
    hasTouch: testInfo.project.use.hasTouch,
  });
  const page = await context.newPage();
  await signInAs(page, account);
  return page;
}

/** A friend of `me` with a name of their own. */
async function friendOf(request: APIRequestContext, me: Account, name = "aoi"): Promise<Account> {
  const friend = await createAccount(request, name);
  await befriend(request, me, friend);
  return friend;
}

/** A room `host` opened through the API; the host is inside on the server and still has to connect. */
async function liveRoom(
  request: APIRequestContext,
  host: Account,
  fields: { title: string; audience?: string; tags?: string[] },
): Promise<Space> {
  const { space } = await rpc<{ space: Space }>(
    request,
    `${SPACES}/CreateSpace`,
    { audience: "SPACE_AUDIENCE_PUBLIC", ...fields },
    host,
  );
  return space;
}

/** Opens a live room's page and goes in (the host comes back to the stage, anyone else listens). */
async function enter(page: Page, space: Space): Promise<void> {
  await page.goto(`/spaces/${space.id}`);
  await page.getByTestId("join-room").click();
  await expect(page.getByTestId("leave-room")).toBeVisible({ timeout: 20_000 });
}

const stage = (page: Page) => page.getByTestId("stage").getByTestId("stage-person");
const audience = (page: Page) => page.getByTestId("audience").getByTestId("audience-person");

// ───────────────────────────── Rooms ─────────────────────────────

test("opening a room from the hub: a friend finds it, listens in and sees the host on stage", async ({
  page,
  request,
  me,
  browser,
}, testInfo) => {
  const friend = await friendOf(request, me);
  const title = `放課後トーク ${Date.now()}`;

  await page.goto("/me");
  await page.getByRole("link", { name: "ルーム", exact: true }).first().click();
  await expect(page.getByRole("heading", { name: "ルーム", level: 1 })).toBeVisible();
  await page.getByTestId("open-room").click();
  await page.getByLabel("なにを話す？").fill(title);
  await page.getByLabel("タグ").fill("#放課後 雑談");
  await page.getByText("だれでも見つけて入れるよ").click();
  await page.getByRole("button", { name: "ルームを開く！" }).click();

  // The host is in their room, on stage, with the microphone still off.
  await expect(page.getByTestId("live-room")).toBeVisible();
  await expect(page.getByRole("heading", { name: title })).toBeVisible();
  await expect(stage(page)).toHaveCount(1);
  await expect(stage(page).first()).toContainText("ホスト");
  await expect(page.getByTestId("mic-toggle")).toHaveAttribute("aria-pressed", "false", {
    timeout: 20_000,
  });
  await page.getByTestId("mic-toggle").click();
  await expect(page.getByTestId("mic-toggle")).toHaveAttribute("aria-pressed", "true");

  const theirs = await browserOf(browser, testInfo, friend);
  await theirs.goto("/spaces");
  // A friend is inside: the room leads the page, and the friend shows as being in a room.
  const card = theirs.getByTestId("space-card").filter({ hasText: title }).first();
  await expect(card).toContainText("LIVE", { timeout: 20_000 });
  await expect(card).toContainText("#放課後");
  await expect(theirs.getByTestId("friends-in-rooms")).toContainText("me");
  await card.getByRole("link", { name: title }).click();

  // From outside: the stage is on show, the audience is not.
  await expect(stage(theirs)).toHaveCount(1);
  await expect(theirs.getByTestId("audience")).toHaveCount(0);
  await theirs.getByTestId("join-room").click();
  await expect(theirs.getByTestId("hand-toggle")).toBeVisible({ timeout: 20_000 });
  await expect(stage(theirs).first()).toContainText("me");
  await expect(stage(theirs).first()).toContainText("ホスト");
  // A listener gets no microphone.
  await expect(theirs.getByTestId("mic-toggle")).toHaveCount(0);

  // The host sees who came in, and the head count follows.
  await expect(audience(page)).toHaveCount(1);
  await expect(audience(page).first()).toContainText("aoi");
  await expect(page.getByTestId("listener-count")).toContainText("2");
  await expect(theirs.getByTestId("listener-count")).toContainText("2");

  // The host's microphone reaches the listener.
  await expect(stage(theirs).first()).toHaveAttribute("data-mic", "on", { timeout: 20_000 });
  await theirs.context().close();
});

test("a raised hand is brought on stage: the listener becomes a speaker and can talk", async ({
  page,
  request,
  me,
  browser,
}, testInfo) => {
  const listener = await createAccount(request, "ren");
  const space = await liveRoom(request, me, { title: "なんでも相談室" });
  await enter(page, space);
  const theirs = await browserOf(browser, testInfo, listener);
  await enter(theirs, space);

  await theirs.getByTestId("hand-toggle").click();
  await expect(theirs.getByTestId("hand-toggle")).toHaveAttribute("aria-pressed", "true");

  // The host sees the hand and brings them up.
  const hands = page.getByTestId("hands");
  await expect(hands).toContainText("ren");
  await hands.getByRole("button", { name: "上げる" }).click();

  // Both see the new speaker on stage.
  await expect(stage(page)).toHaveCount(2);
  await expect(stage(theirs)).toHaveCount(2);
  await expect(stage(page).nth(1)).toContainText("ren");
  await expect(stage(page).nth(1)).toContainText("スピーカー");
  await expect(theirs.getByText("ステージに上がったよ！マイクをオンにして話そう")).toBeVisible();

  // The promotion reached the audio connection: the microphone opens and the host hears it.
  const mic = theirs.getByTestId("mic-toggle");
  await expect(mic).toHaveAttribute("aria-pressed", "false");
  await mic.click();
  await expect(mic).toHaveAttribute("aria-pressed", "true", { timeout: 20_000 });
  // Their microphone reaches the host (whose face lights up when they talk is up to LiveKit's timing).
  await expect(stage(page).nth(1)).toHaveAttribute("data-mic", "on", { timeout: 20_000 });

  // The host mutes them: their own button follows.
  await stage(page).nth(1).click();
  await page.getByRole("button", { name: "ミュートする" }).click();
  await expect(mic).toHaveAttribute("aria-pressed", "false");
  await expect(theirs.getByText(/ホストにミュートされたよ/)).toBeVisible();

  // And sends them back to the audience: the microphone is gone, the hand button is back.
  await stage(page).nth(1).click();
  await page.getByRole("button", { name: "オーディエンスにもどす" }).click();
  await expect(stage(page)).toHaveCount(1);
  await expect(theirs.getByTestId("hand-toggle")).toBeVisible();
  await expect(theirs.getByTestId("mic-toggle")).toHaveCount(0);
  await theirs.context().close();
});

test("an invitation to the stage is a choice: decline it, then accept the next one", async ({
  page,
  request,
  me,
  browser,
}, testInfo) => {
  const listener = await createAccount(request, "mio");
  const space = await liveRoom(request, me, { title: "ステージにおいでよ" });
  await enter(page, space);
  const theirs = await browserOf(browser, testInfo, listener);
  await enter(theirs, space);

  await audience(page).first().click();
  await page.getByRole("button", { name: "ステージに招待する" }).click();
  const invite = theirs.getByTestId("stage-invite");
  await expect(invite).toBeVisible();
  await invite.getByRole("button", { name: "今はやめとく" }).click();
  await expect(invite).toHaveCount(0);
  await expect(stage(page)).toHaveCount(1);

  await audience(page).first().click();
  await page.getByRole("button", { name: "ステージに招待する" }).click();
  await theirs.getByTestId("stage-invite").getByRole("button", { name: "上がる！" }).click();
  await expect(stage(page)).toHaveCount(2);
  await expect(theirs.getByTestId("mic-toggle")).toBeVisible();

  // The host makes them a co-host: they can now moderate too.
  await stage(page).nth(1).click();
  await page.getByRole("button", { name: "共同ホストにする" }).click();
  await expect(stage(theirs).nth(1)).toContainText("共同ホスト");
  await theirs.getByRole("button", { name: "その他" }).click();
  await expect(theirs.getByRole("menuitem", { name: "ピン留め" })).toBeVisible();
  await theirs.keyboard.press("Escape");
  await theirs.context().close();
});

test("an emoji burst floats up for everyone, and a pinned note shows at the top", async ({
  page,
  request,
  me,
  browser,
}, testInfo) => {
  const listener = await createAccount(request, "sora");
  const space = await liveRoom(request, me, { title: "絵文字を投げる会" });
  await enter(page, space);
  const theirs = await browserOf(browser, testInfo, listener);
  await enter(theirs, space);

  await theirs.getByRole("button", { name: "👏を送る" }).click();
  // The sender sees their own at once; the host gets it over the realtime connection.
  await expect(theirs.getByTestId("space-reaction").first()).toContainText("👏");
  await expect(page.getByTestId("space-reaction").first()).toContainText("👏");

  await page.getByRole("button", { name: "その他" }).click();
  await page.getByRole("menuitem", { name: "ピン留め" }).click();
  await page.getByLabel("メモ").fill("今日のテーマは文化祭！");
  await page.getByLabel("リンク").fill("https://example.com/festival");
  await page.getByRole("button", { name: "ピン留めする" }).click();
  const pin = theirs.getByTestId("room-pin");
  await expect(pin).toContainText("今日のテーマは文化祭！");
  await expect(pin.getByRole("link")).toHaveAttribute("href", "https://example.com/festival");
  await theirs.context().close();
});

test("the host removes a listener: they land on a clear notice and are not reconnected", async ({
  page,
  request,
  me,
  browser,
}, testInfo) => {
  const listener = await createAccount(request, "kai");
  const space = await liveRoom(request, me, { title: "しずかにしてね" });
  await enter(page, space);
  const theirs = await browserOf(browser, testInfo, listener);
  await enter(theirs, space);

  await audience(page).first().click();
  await page.getByRole("button", { name: "退出させる" }).click();
  await page.getByRole("button", { name: "退出させる" }).last().click();

  const notice = theirs.getByTestId("exit-notice");
  await expect(notice).toHaveAttribute("data-reason", "removed");
  await expect(notice).toContainText("ルームから退出になったよ");
  await expect(audience(page)).toHaveCount(0);
  await expect(page.getByTestId("listener-count")).toContainText("1");
  // No way back in happens by itself: a while later they are still outside.
  await theirs.waitForTimeout(6000);
  await expect(notice).toBeVisible();
  await expect(theirs.getByTestId("mini-player")).toHaveCount(0);
  await expect(page.getByTestId("listener-count")).toContainText("1");

  // Removed is not banned: they may come back in themselves.
  await notice.getByRole("button", { name: "もう一度入る" }).click();
  await expect(theirs.getByTestId("leave-room")).toBeVisible({ timeout: 20_000 });
  await expect(audience(page)).toHaveCount(1);

  // A ban is final: the room is gone for them, also after a reload.
  await audience(page).first().click();
  await page.getByRole("button", { name: "BANする" }).click();
  await page.getByRole("button", { name: "BANする" }).last().click();
  await expect(theirs.getByTestId("exit-notice")).toHaveAttribute("data-reason", "banned");
  await expect(theirs.getByTestId("exit-notice")).toContainText("このルームには入れなくなったよ");
  await expect(theirs.getByRole("button", { name: "もう一度入る" })).toHaveCount(0);
  await theirs.reload();
  await expect(theirs.getByText("このルームは見つからないよ")).toBeVisible();

  // The host finds them under "banned" and can lift it.
  await page.getByRole("button", { name: "その他" }).click();
  await page.getByRole("menuitem", { name: "BANした人" }).click();
  await expect(page.getByRole("dialog")).toContainText("kai");
  await page.getByRole("button", { name: "解除する" }).click();
  await expect(page.getByRole("dialog")).toContainText("BANした人はいないよ");
  await theirs.context().close();
});

test("a listener who keeps their online status private is counted, not named", async ({
  page,
  request,
  // `me` signs this browser in as a fresh account.
  me: _me,
  browser,
}, testInfo) => {
  const shy = await createAccount(request, "shy");
  const { settings } = await rpc<{ settings: { privacy: object } }>(
    request,
    "tokuly.identity.v1.AccountService/GetSettings",
    {},
    shy,
  );
  await rpc(
    request,
    "tokuly.identity.v1.AccountService/UpdateSettings",
    { privacy: { ...settings.privacy, shareOnlineStatus: false } },
    shy,
  );
  const open = await createAccount(request, "yuu");
  const host = await createAccount(request, "host");
  const space = await liveRoom(request, host, { title: "こっそり聴ける部屋" });
  await rpc(request, `${SPACES}/JoinSpace`, { spaceId: space.id }, shy);
  await rpc(request, `${SPACES}/JoinSpace`, { spaceId: space.id }, open);

  // Another listener sees the one who shares, and only a number for the one who does not.
  await enter(page, space);
  await expect(page.getByTestId("listener-count")).toContainText("4");
  await expect(audience(page).filter({ hasText: "yuu" })).toHaveCount(1);
  await expect(page.getByTestId("audience")).not.toContainText("shy");
  await expect(page.getByTestId("hidden-count")).toHaveText("ほかに1人がこっそり聴いてるよ");

  // Moderators see everyone.
  const hosts = await browserOf(browser, testInfo, host);
  await enter(hosts, space);
  await expect(audience(hosts).filter({ hasText: "shy" })).toHaveCount(1);
  await expect(hosts.getByTestId("hidden-count")).toHaveCount(0);
  await hosts.context().close();
});

test("the mini player keeps the room going while moving around the app", async ({
  page,
  request,
  // `me` signs this browser in as a fresh account.
  me: _me,
  browser,
}, testInfo) => {
  const host = await createAccount(request, "hina");
  const space = await liveRoom(request, host, { title: "ながら聴きラジオ" });
  const hosts = await browserOf(browser, testInfo, host);
  await enter(hosts, space);
  await hosts.getByTestId("mic-toggle").click();
  await enter(page, space);
  await expect(hosts.getByTestId("listener-count")).toContainText("2");

  // Out of the room's page: the pill takes over.
  await page.getByRole("link", { name: "ルームを小さくして一覧へ" }).click();
  await expect(page.getByRole("heading", { name: "ルーム", level: 1 })).toBeVisible();
  const pill = page.getByTestId("mini-player");
  await expect(pill).toContainText("ながら聴きラジオ");
  await expect(pill).toHaveAttribute("data-phase", "connected");

  // Further away, on other features' pages, the room is still on.
  await page.getByRole("link", { name: "back" }).click();
  await expect(page).toHaveURL(/\/me$/);
  await expect(pill).toBeVisible();
  await page.getByRole("link", { name: "ともだち", exact: true }).first().click();
  await expect(page).toHaveURL(/\/friends/);
  await expect(pill).toHaveAttribute("data-phase", "connected");
  await expect(hosts.getByTestId("listener-count")).toContainText("2");

  // The pill leads back into the room, where the page is the player again.
  await pill.getByRole("link", { name: "「ながら聴きラジオ」を開く" }).click();
  await expect(page.getByTestId("leave-room")).toBeVisible();
  await expect(pill).toHaveCount(0);

  // And it can leave from anywhere.
  await page.getByRole("link", { name: "ルームを小さくして一覧へ" }).click();
  await pill.getByRole("button", { name: "抜ける" }).click();
  await expect(pill).toHaveCount(0);
  await expect(hosts.getByTestId("listener-count")).toContainText("1");
  await hosts.context().close();
});

test("the host ends the room: listeners land on its summary", async ({
  page,
  request,
  me,
  browser,
}, testInfo) => {
  const listener = await createAccount(request, "nagi");
  const space = await liveRoom(request, me, { title: "おわりの会" });
  await enter(page, space);
  const theirs = await browserOf(browser, testInfo, listener);
  await enter(theirs, space);

  // Leaving as the host asks what should become of the room.
  await page.getByTestId("leave-room").click();
  await page.getByRole("button", { name: "ルームを終了する" }).click();
  await expect(page.getByTestId("ended-room")).toContainText("ホストがルームをおわりにしたよ");
  await expect(page.getByTestId("mini-player")).toHaveCount(0);

  await expect(theirs.getByTestId("ended-room")).toContainText("おわりの会");
  await expect(theirs.getByText("ルームがおわったよ")).toBeVisible();
  await expect(theirs.getByTestId("mini-player")).toHaveCount(0);
  // Who was on stage stays on record for the people who were there.
  await expect(theirs.getByText("いた人たち")).toBeVisible();

  // The host clears it from their history.
  await page.getByRole("button", { name: "削除" }).click();
  await page.getByRole("button", { name: "削除" }).last().click();
  await expect(page).toHaveURL(/\/spaces$/);
  await theirs.context().close();
});

test("a scheduled room: a friend sets a reminder and is taken along when it starts", async ({
  page,
  request,
  me,
  browser,
}, testInfo) => {
  const friend = await friendOf(request, me, "yuna");
  const title = `テスト前夜の勉強会 ${Date.now()}`;

  await page.goto("/spaces?tab=mine");
  await page.getByRole("button", { name: "ルームを予約する" }).first().click();
  await page.getByLabel("なにを話す？").fill(title);
  // The form proposes the next full hour on the account's clock.
  await expect(page.getByLabel("日にち")).not.toHaveValue("");
  await page.getByRole("button", { name: "予約する！" }).click();
  await expect(page.getByTestId("scheduled-room")).toContainText(title);
  await expect(page.getByTestId("scheduled-room")).toContainText(/今日|あした/);

  const theirs = await browserOf(browser, testInfo, friend);
  await theirs.goto("/spaces");
  const card = theirs.getByTestId("space-card").filter({ hasText: title });
  await card.getByRole("button", { name: "はじまったら教えて" }).click();
  await expect(card.getByRole("button", { name: "リマインド中" })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  // It is among "my reminders", and the room's own page knows too.
  await theirs.getByRole("button", { name: "リマインドしたのだけ" }).click();
  await expect(theirs.getByTestId("space-card").filter({ hasText: title })).toHaveCount(1);
  await theirs.getByRole("link", { name: title }).click();
  await expect(theirs.getByTestId("scheduled-room")).toContainText("1人がたのしみにしてるよ");

  // The host starts it: the friend's page turns into the live room, one tap from listening.
  await page.reload();
  await expect(page.getByTestId("reminder-count")).toHaveText("1人がたのしみにしてるよ");
  await page.getByRole("button", { name: "はじめる" }).click();
  await expect(page.getByTestId("leave-room")).toBeVisible({ timeout: 20_000 });
  await expect(theirs.getByTestId("join-room")).toBeVisible();
  // The reminder arrived as a notification.
  await expect(async () => {
    const { notifications } = await rpc<{ notifications?: { kind: string }[] }>(
      request,
      "tokuly.notify.v1.InboxService/ListNotifications",
      {},
      friend,
    );
    expect(notifications?.map((entry) => entry.kind)).toContain("spaces.reminder");
  }).toPass({ timeout: 20_000 });
  await theirs.context().close();
});

test("an upcoming room can be changed and called off by its host", async ({
  page,
  request,
  me,
}) => {
  const { space } = await rpc<{ space: Space }>(
    request,
    `${SPACES}/CreateSpace`,
    {
      title: "まだ決まってない会",
      audience: "SPACE_AUDIENCE_FRIENDS",
      scheduledAt: new Date(Date.now() + 3 * 3_600_000).toISOString(),
    },
    me,
  );
  await page.goto("/spaces?tab=mine");
  const card = page.getByTestId("space-card").filter({ hasText: "まだ決まってない会" });
  await card.getByRole("button", { name: "編集" }).click();
  await page.getByLabel("なにを話す？").fill("修学旅行の作戦会議");
  await page.getByLabel("タグ").fill("#修学旅行");
  await page.getByRole("button", { name: "保存する" }).click();
  const renamed = page.getByTestId("space-card").filter({ hasText: "修学旅行の作戦会議" });
  await expect(renamed).toContainText("#修学旅行");

  await renamed.getByRole("button", { name: "中止する" }).click();
  await page.getByRole("button", { name: "中止する" }).last().click();
  // Called off: nothing is left to start or change, only to delete.
  await expect(renamed.getByRole("button", { name: "はじめる" })).toHaveCount(0);
  await expect(renamed.getByRole("button", { name: "削除" })).toBeVisible();
  const after = await rpc<{ space: Space }>(
    request,
    `${SPACES}/GetSpace`,
    { spaceId: space.id },
    me,
  );
  expect(after.space.state).toBe("SPACE_STATE_CANCELLED");
});

// ───────────────────────────── たまり場 ─────────────────────────────

test("a たまり場 shows who is inside: created, joined by a friend, entered and left", async ({
  page,
  request,
  me,
  browser,
}, testInfo) => {
  const friend = await friendOf(request, me, "rio");

  await page.goto("/spaces?tab=hangouts");
  await page.getByRole("button", { name: "たまり場をつくる" }).click();
  await page.getByLabel("なまえ").fill("いつメン");
  await page.getByRole("button", { name: "ともだちをさそう" }).click();
  await page.getByRole("button", { name: /rio/ }).click();
  await page.getByRole("button", { name: "これでOK" }).click();
  await page.getByRole("button", { name: "つくる！" }).click();
  await expect(page).toHaveURL(/\/spaces\/hangouts\//);
  await expect(page.getByTestId("hangout-room")).toContainText("今はだれもいないよ");
  const hangoutId = page.url().split("/").pop() as string;

  // The friend finds the invitation (counted on the tab) and joins the group.
  const theirs = await browserOf(browser, testInfo, friend);
  await theirs.goto("/spaces");
  await expect(theirs.getByRole("tab", { name: /たまり場/ })).toContainText("1", {
    timeout: 20_000,
  });
  await theirs.getByRole("tab", { name: /たまり場/ }).click();
  const invitation = theirs.getByTestId("hangout-invitations");
  await expect(invitation).toContainText("いつメン");
  await invitation.getByRole("button", { name: "入る！" }).click();
  const card = theirs.getByTestId("hangout-card").filter({ hasText: "いつメン" });
  await expect(card).toContainText("今はだれもいないよ");
  await expect(page.getByTestId("hangout-members")).toContainText("rio");

  // `me` opens the room: the friend's list says so without a reload.
  await page.getByTestId("join-hangout").click();
  await expect(page.getByTestId("leave-hangout")).toBeVisible({ timeout: 20_000 });
  await expect(page.getByTestId("mic-toggle")).toHaveAttribute("aria-pressed", "true");
  await expect(card).toContainText("今1人いるよ");
  await expect(card).toHaveAttribute("data-open", "true");

  // On the hangout's page the friend sees who it is, and on the discover tab too.
  await card.click();
  await expect(theirs.getByTestId("hangout-inside")).toContainText("me");
  await theirs.getByTestId("join-hangout").click();
  await expect(theirs.getByTestId("leave-hangout")).toBeVisible({ timeout: 20_000 });
  await expect(page.getByTestId("hangout-inside-count")).toHaveText("今2人いるよ");
  await expect(page.getByTestId("hangout-inside")).toContainText("rio");
  // Everyone in a hangout talks: the friend's microphone reaches `me`.
  await expect(
    page.getByTestId("hangout-inside").locator("li").filter({ hasText: "rio" }),
  ).toHaveAttribute("data-mic", "on", { timeout: 20_000 });

  await theirs.getByTestId("leave-hangout").click();
  await expect(page.getByTestId("hangout-inside-count")).toHaveText("今1人いるよ");
  await theirs.getByRole("link", { name: "back" }).click();
  await expect(theirs.getByTestId("friends-in-rooms")).toContainText("いつメン");

  // The address the server uses for a hangout leads to the same page.
  await theirs.goto(`/hangouts/${hangoutId}`);
  await expect(theirs).toHaveURL(new RegExp(`/spaces/hangouts/${hangoutId}$`));
  await expect(theirs.getByTestId("hangout-inside")).toContainText("me");
  await theirs.context().close();
});

test("a hangout's owner renames it and removes a member, who is cut off at once", async ({
  page,
  request,
  me,
  browser,
}, testInfo) => {
  const friend = await friendOf(request, me, "taku");
  const { hangout } = await rpc<{ hangout: { id: string } }>(
    request,
    `${HANGOUTS}/CreateHangout`,
    { name: "部室", emoji: "🎸", membersCanInvite: true, inviteeIds: [friend.userId] },
    me,
  );
  await rpc(
    request,
    `${HANGOUTS}/RespondToHangoutInvite`,
    { hangoutId: hangout.id, accept: true },
    friend,
  );

  const theirs = await browserOf(browser, testInfo, friend);
  await theirs.goto(`/spaces/hangouts/${hangout.id}`);
  await theirs.getByTestId("join-hangout").click();
  await expect(theirs.getByTestId("leave-hangout")).toBeVisible({ timeout: 20_000 });

  await page.goto(`/spaces/hangouts/${hangout.id}`);
  await expect(page.getByTestId("hangout-inside")).toContainText("taku");
  await page.getByRole("button", { name: "その他" }).click();
  await page.getByRole("menuitem", { name: "たまり場の設定" }).click();
  await page.getByLabel("なまえ").fill("軽音部の部室");
  await page.getByRole("button", { name: "保存する" }).click();
  await expect(page.getByRole("heading", { level: 1 })).toContainText("軽音部の部室");
  await expect(theirs.getByRole("heading", { level: 1 })).toContainText("軽音部の部室");

  await page.getByRole("button", { name: "takuの操作" }).click();
  await page.getByRole("menuitem", { name: "メンバーからはずす" }).click();
  await page.getByRole("button", { name: "メンバーからはずす" }).click();
  await expect(page.getByTestId("hangout-members")).not.toContainText("taku");

  // They are out, told why, and nothing tries to get back in.
  const notice = theirs.getByTestId("exit-notice");
  await expect(notice).toHaveAttribute("data-reason", "closed");
  await expect(notice).toContainText("このたまり場はもうないよ");
  await expect(theirs.getByTestId("mini-player")).toHaveCount(0);
  await theirs.context().close();
});
