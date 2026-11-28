import { createECDH, randomBytes } from "node:crypto";
import type { APIRequestContext, Locator, Page } from "@playwright/test";
import { postMoment, promptReached } from "./moments";
import { discPicture, uploadPicture } from "./pictures";
import {
  type Account,
  expect as baseExpect,
  befriend,
  createAccount,
  failToLoadPart,
  rpc,
  signInAs,
  test,
} from "./support";

// These tests share a machine with many other builds and browsers: a cold page
// or a busy API can take a while to answer. Waiting longer costs nothing when
// things are quick.
const expect = baseExpect.configure({ timeout: 20_000 });
test.describe.configure({ timeout: 90_000 });

const INBOX = "tokuly.notify.v1.InboxService";
const PREFERENCES = "tokuly.notify.v1.PreferencesService";
const PUSH = "tokuly.notify.v1.PushService";

// ── Arranging and checking through the API ──────────────────────────

/** An inbox entry as the JSON API returns it (unset fields are left out). */
interface Entry {
  id: string;
  kind: string;
  title: string;
  body: string;
  path: string;
  read?: boolean;
  groupCount: number;
  actor?: { handle: string };
}

async function inboxOf(request: APIRequestContext, as: Account): Promise<Entry[]> {
  return (
    (await rpc<{ notifications?: Entry[] }>(request, `${INBOX}/ListNotifications`, {}, as))
      .notifications ?? []
  );
}

async function unreadOf(request: APIRequestContext, as: Account): Promise<number> {
  return (
    (await rpc<{ unread?: { total?: number } }>(request, `${INBOX}/GetUnreadCount`, {}, as)).unread
      ?.total ?? 0
  );
}

/** Notifications are produced from events, a moment after the action that caused them. */
async function expectUnread(request: APIRequestContext, as: Account, count: number): Promise<void> {
  await expect.poll(() => unreadOf(request, as), { timeout: 20_000 }).toBe(count);
}

/** A new account called `name` sends `to` a friend request: one "友達リクエスト" entry in `to`'s inbox. */
async function friendRequestFrom(
  request: APIRequestContext,
  name: string,
  to: Account,
): Promise<Account> {
  const sender = await createAccount(request, name);
  await rpc(
    request,
    "tokuly.social.v1.FriendService/SendFriendRequest",
    { userId: to.userId },
    sender,
  );
  return sender;
}

/** Two friend requests for `me`, from "haru" (older) and "sora" (newer), both arrived. */
async function twoRequests(
  request: APIRequestContext,
  me: Account,
): Promise<{ haru: Account; sora: Account }> {
  const haru = await friendRequestFrom(request, "haru", me);
  await expectUnread(request, me, 1);
  const sora = await friendRequestFrom(request, "sora", me);
  await expectUnread(request, me, 2);
  return { haru, sora };
}

async function markReadThroughApi(
  request: APIRequestContext,
  me: Account,
  from: Account,
): Promise<void> {
  const entry = (await inboxOf(request, me)).find(
    (candidate) => candidate.actor?.handle === from.handle,
  );
  await rpc(request, `${INBOX}/MarkRead`, { ids: { ids: [entry?.id] } }, me);
}

interface Preferences {
  categories: { category: string; inbox?: boolean; push?: boolean }[];
  quietHours?: { enabled?: boolean; startMinute?: number; endMinute?: number };
  pushPause?: { paused?: boolean; until?: string };
  timeZone: string;
  pushPreview?: string;
}

async function preferencesOf(request: APIRequestContext, as: Account): Promise<Preferences> {
  return (await rpc<{ preferences: Preferences }>(request, `${PREFERENCES}/GetPreferences`, {}, as))
    .preferences;
}

async function devicesOf(
  request: APIRequestContext,
  as: Account,
): Promise<{ deviceName: string }[]> {
  return (
    (await rpc<{ devices?: { deviceName: string }[] }>(request, `${PUSH}/ListDevices`, {}, as))
      .devices ?? []
  );
}

// ── Finding things on the page ──────────────────────────────────────

/** The inbox entry (or any list row) containing `text`. */
const row = (page: Page, text: string | RegExp): Locator =>
  page.getByRole("listitem").filter({ hasText: text });

/** The unread counter on the navigation: on the bell (rail) or on "じぶん" (phone tab bar). */
const navBadge = (page: Page): Locator =>
  page.getByRole("navigation", { name: "main" }).getByText(/^\d+\+?$/);

const toasts = (page: Page): Locator => page.locator("[data-sonner-toast]");

/**
 * Resolves once the page's realtime connection is up, so that nothing sent
 * afterwards is missed. Development builds expose the connection; elsewhere
 * it gets a moment.
 */
async function live(page: Page): Promise<void> {
  await expect(page.getByRole("navigation", { name: "main" })).toBeVisible();
  const connected = () => {
    const connection = (window as { __tokulyRealtime?: { connectionStatus: string } })
      .__tokulyRealtime;
    return connection ? connection.connectionStatus === "connected" : null;
  };
  if ((await page.evaluate(connected)) === null) await page.waitForTimeout(3000);
  else await page.waitForFunction(connected, null, { timeout: 20_000 });
}

/** A leftward swipe with a finger across `target` (real touch input, through the browser). */
async function swipeLeft(page: Page, target: Locator): Promise<void> {
  const box = await target.boundingBox();
  if (!box) throw new Error("nothing to swipe");
  const session = await page.context().newCDPSession(page);
  const y = Math.round(box.y + box.height / 2);
  let x = Math.round(box.x + box.width - 40);
  await session.send("Input.dispatchTouchEvent", { type: "touchStart", touchPoints: [{ x, y }] });
  for (let step = 0; step < 10; step++) {
    x -= 22;
    await session.send("Input.dispatchTouchEvent", { type: "touchMove", touchPoints: [{ x, y }] });
  }
  await session.send("Input.dispatchTouchEvent", { type: "touchEnd", touchPoints: [] });
  await session.detach();
}

/**
 * Headless Chrome has no push service: subscribing fails. The browser's half
 * is played by a stand-in subscription with well-formed keys, whose endpoint
 * is on a push host the server accepts but that does not exist, so the
 * registration is real and nothing is ever delivered anywhere.
 */
async function fakePushService(page: Page): Promise<void> {
  const ecdh = createECDH("prime256v1");
  ecdh.generateKeys();
  const fake = {
    endpoint: `https://e2e-${randomBytes(6).toString("hex")}.invalid.notify.windows.com/w/?token=${randomBytes(12).toString("hex")}`,
    keys: {
      p256dh: ecdh.getPublicKey().toString("base64url"),
      auth: randomBytes(16).toString("base64url"),
    },
  };
  await page.addInitScript((subscription) => {
    const STORE = "e2e.push.serverKey";
    const bytes = (base64url: string) =>
      Uint8Array.from(atob(base64url.replace(/-/g, "+").replace(/_/g, "/")), (char) =>
        char.charCodeAt(0),
      ).buffer;
    const make = (serverKey: number[]) =>
      ({
        endpoint: subscription.endpoint,
        expirationTime: null,
        options: { userVisibleOnly: true, applicationServerKey: new Uint8Array(serverKey).buffer },
        getKey: (name: "p256dh" | "auth") => bytes(subscription.keys[name]),
        toJSON: () => ({
          endpoint: subscription.endpoint,
          expirationTime: null,
          keys: subscription.keys,
        }),
        unsubscribe: async () => {
          window.localStorage.removeItem(STORE);
          return true;
        },
      }) as unknown as PushSubscription;
    PushManager.prototype.subscribe = async (options) => {
      const serverKey = Array.from(new Uint8Array(options?.applicationServerKey as ArrayBuffer));
      window.localStorage.setItem(STORE, JSON.stringify(serverKey));
      return make(serverKey);
    };
    PushManager.prototype.getSubscription = async () => {
      const stored = window.localStorage.getItem(STORE);
      return stored ? make(JSON.parse(stored) as number[]) : null;
    };
  }, fake);
}

// ── Inbox ───────────────────────────────────────────────────────────

test("the inbox shows who did what, newest first, and the navigation counts what is unread", async ({
  page,
  request,
  me,
}) => {
  await twoRequests(request, me);

  await page.goto("/notifications");
  await expect(page.getByRole("heading", { name: "お知らせ", exact: true })).toBeVisible();
  const rows = row(page, "友達リクエストが届いたよ");
  await expect(rows).toHaveCount(2);
  await expect(rows.nth(0)).toContainText("soraさんから友達リクエストが届いたよ");
  await expect(rows.nth(1)).toContainText("haruさんから友達リクエストが届いたよ");
  await expect(rows.nth(0).getByText("未読", { exact: true })).toBeAttached();
  await expect(page.getByRole("heading", { name: "今日" })).toBeVisible();

  await expect(navBadge(page)).toHaveText("2");
  await expect(page.getByRole("tab", { name: /未読/ })).toContainText("2");
  await expect(page.getByRole("tab", { name: /友達/ })).toContainText("2");
});

test("an empty inbox says so and points to finding friends", async ({ page, me: _me }) => {
  await page.goto("/notifications");
  await expect(page.getByText("お知らせはまだないよ")).toBeVisible();
  await expect(page.getByRole("link", { name: "ともだちをさがす" })).toHaveAttribute(
    "href",
    "/friends",
  );
  await expect(navBadge(page)).toHaveCount(0);
});

test("opening an entry marks it read and goes where it points", async ({ page, request, me }) => {
  const haru = await friendRequestFrom(request, "haru", me);
  await expectUnread(request, me, 1);

  await page.goto("/notifications");
  await row(page, "haruさんから").getByRole("link").click();
  await expect(page).toHaveURL(new RegExp(`/u/${haru.handle}$`));
  await expectUnread(request, me, 0);
  await expect(navBadge(page)).toHaveCount(0);

  await page.goBack();
  await expect(row(page, "haruさんから")).toBeVisible();
  await expect(row(page, "haruさんから").getByText("未読", { exact: true })).toHaveCount(0);
});

test("a RealMoji on my moment arrives in the inbox and opens the moment", async ({
  page,
  request,
  me,
}) => {
  const aoi = await createAccount(request, "aoi");
  await befriend(request, me, aoi);
  promptReached([me, aoi]);
  const moment = await postMoment(request, me);
  await postMoment(request, aoi, { color: [40, 160, 220] });
  const selfie = await uploadPicture(
    request,
    aoi,
    discPicture("selfie.png", [250, 200, 60]),
    "ASSET_PURPOSE_REALMOJI",
  );
  const REACTIONS = "tokuly.moments.v1.MomentReactionService";
  await rpc(
    request,
    `${REACTIONS}/SetRealmoji`,
    { emoji: "REALMOJI_EMOJI_SMILE", assetId: selfie },
    aoi,
  );
  await rpc(
    request,
    `${REACTIONS}/React`,
    { momentId: moment, realmoji: "REALMOJI_EMOJI_SMILE" },
    aoi,
  );
  await expect
    .poll(async () => (await inboxOf(request, me)).map((entry) => entry.title))
    .toContain("aoiさんがリアクションしたよ!");

  await page.goto("/notifications");
  await row(page, "aoiさんがリアクションしたよ!").getByRole("link").click();
  await expect(page).toHaveURL(new RegExp(`/moments/${moment}$`));
  await expect(page.getByTestId("moment-reaction-list").getByText("aoi")).toBeVisible();
});

test("a new notification pops up as a toast wherever you are, and the toast opens it", async ({
  page,
  request,
  me,
}) => {
  await page.goto("/me");
  await live(page);
  const haru = await friendRequestFrom(request, "haru", me);

  const toast = toasts(page).filter({ hasText: "haruさんから友達リクエストが届いたよ" });
  await expect(toast).toBeVisible();
  await expect(navBadge(page)).toHaveText("1");

  await toast.getByRole("button", { name: /友達リクエスト/ }).click();
  await expect(page).toHaveURL(new RegExp(`/u/${haru.handle}$`));
  await expectUnread(request, me, 0);
  await expect(navBadge(page)).toHaveCount(0);
});

test("a toast whose opening code cannot load says so, stays, and opens when tapped again", async ({
  page,
  request,
  me,
}) => {
  await page.goto("/me");
  await live(page);
  // What opens a notification fails to arrive, as when the connection drops right as it is tapped.
  const letThrough = await failToLoadPart(page, "notifications", "inbox-actions.ts");
  const haru = await friendRequestFrom(request, "haru", me);

  const toast = toasts(page).filter({ hasText: "haruさんから友達リクエストが届いたよ" });
  await toast.getByRole("button", { name: /友達リクエスト/ }).click();
  await expect(
    toasts(page).filter({ hasText: "通信できなかったよ。接続を確認してね!" }),
  ).toBeVisible();
  await expect(page).toHaveURL(/\/me$/);

  await letThrough();
  await toast.getByRole("button", { name: /友達リクエスト/ }).click();
  await expect(page).toHaveURL(new RegExp(`/u/${haru.handle}$`));
});

test("the open inbox updates by itself instead of showing a toast", async ({
  page,
  request,
  me,
}) => {
  await page.goto("/notifications");
  await expect(page.getByText("お知らせはまだないよ")).toBeVisible();
  await live(page);

  await friendRequestFrom(request, "haru", me);
  await expect(row(page, "haruさんから友達リクエストが届いたよ")).toBeVisible();
  await expect(navBadge(page)).toHaveText("1");
  await expect(toasts(page)).toHaveCount(0);
});

test("everything can be marked read at once", async ({ page, request, me }) => {
  await twoRequests(request, me);

  await page.goto("/notifications");
  await expect(row(page, "友達リクエストが届いたよ")).toHaveCount(2);
  await page.getByRole("button", { name: "ぜんぶ既読にする" }).click();
  await expect(page.getByText("2件を既読にしたよ")).toBeVisible();
  await expect(page.getByRole("listitem").getByText("未読", { exact: true })).toHaveCount(0);
  await expect(navBadge(page)).toHaveCount(0);
  await expect(page.getByRole("button", { name: "ぜんぶ既読にする" })).toBeDisabled();
  await expectUnread(request, me, 0);
});

test("an entry's menu marks it read or deletes it", async ({ page, request, me }) => {
  const { sora } = await twoRequests(request, me);

  await page.goto("/notifications");
  await row(page, "soraさんから").getByRole("button", { name: "メニュー" }).click();
  await page.getByRole("menuitem", { name: "既読にする" }).click();
  await expect(row(page, "soraさんから").getByText("未読", { exact: true })).toHaveCount(0);
  await expect(navBadge(page)).toHaveText("1");
  await expectUnread(request, me, 1);

  await row(page, "haruさんから").getByRole("button", { name: "メニュー" }).click();
  await page.getByRole("menuitem", { name: "削除" }).click();
  await expect(row(page, "haruさんから")).toHaveCount(0);
  await expect(row(page, "soraさんから")).toBeVisible();
  await expect(navBadge(page)).toHaveCount(0);
  await expect
    .poll(async () => (await inboxOf(request, me)).map((entry) => entry.actor?.handle))
    .toEqual([sora.handle]);
});

test("on a phone, swiping an entry away deletes it", async ({ page, request, me }, testInfo) => {
  test.skip(testInfo.project.name !== "phone", "swiping needs a touch screen");
  const { sora } = await twoRequests(request, me);

  await page.goto("/notifications");
  await expect(row(page, "haruさんから")).toBeVisible();
  await swipeLeft(page, row(page, "haruさんから"));
  await expect(row(page, "haruさんから")).toHaveCount(0);
  // The swipe did not also open the entry.
  await expect(page).toHaveURL(/\/notifications$/);
  await expect
    .poll(async () => (await inboxOf(request, me)).map((entry) => entry.actor?.handle))
    .toEqual([sora.handle]);
});

test("read entries, or all of them, are cleared after a confirmation", async ({
  page,
  request,
  me,
}) => {
  const { haru, sora } = await twoRequests(request, me);
  await markReadThroughApi(request, me, haru);

  await page.goto("/notifications");
  await expect(row(page, "友達リクエストが届いたよ")).toHaveCount(2);

  await page.getByRole("button", { name: "その他" }).click();
  await page.getByRole("menuitem", { name: "既読のお知らせを削除" }).click();
  await expect(page.getByRole("dialog")).toContainText("既読のお知らせを削除する？");
  await page.getByRole("dialog").getByRole("button", { name: "削除する" }).click();
  await expect(page.getByText("1件のお知らせを削除したよ")).toBeVisible();
  await expect(row(page, "haruさんから")).toHaveCount(0);
  await expect(row(page, "soraさんから")).toBeVisible();
  await expect
    .poll(async () => (await inboxOf(request, me)).map((entry) => entry.actor?.handle))
    .toEqual([sora.handle]);

  // Changing your mind keeps everything.
  await page.getByRole("button", { name: "その他" }).click();
  await page.getByRole("menuitem", { name: "すべて削除" }).click();
  await expect(page.getByRole("dialog")).toContainText("お知らせをぜんぶ削除する？");
  await page.getByRole("dialog").getByRole("button", { name: "キャンセル" }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(row(page, "soraさんから")).toBeVisible();

  await page.getByRole("button", { name: "その他" }).click();
  await page.getByRole("menuitem", { name: "すべて削除" }).click();
  await page.getByRole("dialog").getByRole("button", { name: "削除する" }).click();
  await expect(page.getByText("お知らせはまだないよ")).toBeVisible();
  await expect(navBadge(page)).toHaveCount(0);
  await expect.poll(() => inboxOf(request, me)).toEqual([]);
});

test("the tabs filter by unread and by category", async ({ page, request, me }) => {
  const { haru } = await twoRequests(request, me);
  await markReadThroughApi(request, me, haru);

  await page.goto("/notifications");
  await expect(row(page, "友達リクエストが届いたよ")).toHaveCount(2);

  await page.getByRole("tab", { name: /未読/ }).click();
  await expect(page).toHaveURL(/tab=unread/);
  await expect(row(page, "soraさんから")).toBeVisible();
  await expect(row(page, "haruさんから")).toHaveCount(0);

  await page.getByRole("tab", { name: /チャット/ }).click();
  await expect(page.getByText("「チャット・通話」のお知らせはないよ")).toBeVisible();

  await page.getByRole("tab", { name: /友達/ }).click();
  await expect(row(page, "友達リクエストが届いたよ")).toHaveCount(2);

  // The chosen tab is part of the address, so it survives a reload (and going back).
  await page.getByRole("tab", { name: /未読/ }).click();
  await expect(row(page, "haruさんから")).toHaveCount(0);
  await page.reload();
  await expect(page.getByRole("tab", { name: /未読/ })).toHaveAttribute("aria-selected", "true");
  await expect(row(page, "soraさんから")).toBeVisible();
  await expect(row(page, "haruさんから")).toHaveCount(0);
});

test("events the server folded into one entry show how many there were", async ({
  page,
  request,
  me,
}) => {
  const yuto = await createAccount(request, "yuto");
  await befriend(request, me, yuto);
  for (let knock = 0; knock < 3; knock++) {
    await rpc(
      request,
      "tokuly.social.v1.InteractionService/Knock",
      { userId: me.userId, count: 1 },
      yuto,
    );
  }
  await expect
    .poll(
      async () =>
        (await inboxOf(request, me)).find((entry) => entry.kind === "social.knock")?.groupCount,
      { timeout: 20_000 },
    )
    .toBe(3);

  await page.goto("/notifications");
  await expect(row(page, "yutoさんがノックしてるよ")).toHaveCount(1);
  await expect(row(page, "yutoさんがノックしてるよ").getByText("ほか2件")).toBeVisible();
});

test("older entries load as you scroll", async ({ page, request, me }) => {
  test.setTimeout(120_000);
  const total = 23; // one page is 20
  for (let first = 0; first < total; first += 6) {
    await Promise.all(
      Array.from({ length: Math.min(6, total - first) }, (_, offset) =>
        friendRequestFrom(request, `p${first + offset}`, me),
      ),
    );
  }
  await expectUnread(request, me, total);

  await page.goto("/notifications");
  const rows = row(page, "友達リクエストが届いたよ");
  await expect(rows).toHaveCount(20);
  await rows.last().scrollIntoViewIfNeeded();
  await expect(rows).toHaveCount(total);
});

// ── Preferences ─────────────────────────────────────────────────────

test("the inbox leads to the settings and back", async ({ page, me: _me }) => {
  await page.goto("/notifications");
  await page.getByRole("link", { name: "通知設定" }).click();
  await expect(page).toHaveURL(/\/notifications\/settings$/);
  await expect(page.getByRole("heading", { name: "通知設定" })).toBeVisible();
  await page.getByRole("link", { name: "back" }).click();
  await expect(page).toHaveURL(/\/notifications$/);
});

test("category switches are saved", async ({ page, request, me }) => {
  await page.goto("/notifications/settings");
  const chatPush = page.getByRole("switch", { name: "チャット・通話をプッシュ通知する" });
  const socialInbox = page.getByRole("switch", { name: "友達をお知らせに残す" });
  await expect(chatPush).toBeChecked();
  await expect(socialInbox).toBeChecked();
  // Account and security notices always stay in the inbox.
  await expect(
    page.getByRole("switch", { name: "アカウント・セキュリティをお知らせに残す" }),
  ).toBeDisabled();

  await chatPush.click();
  await socialInbox.click();
  await expect(chatPush).not.toBeChecked();
  await expect(socialInbox).not.toBeChecked();
  const saved = async () => {
    const categories = (await preferencesOf(request, me)).categories;
    const of = (id: string) => categories.find((category) => category.category === id);
    return {
      chatInbox: Boolean(of("chat")?.inbox),
      chatPush: Boolean(of("chat")?.push),
      socialInbox: Boolean(of("social")?.inbox),
      socialPush: Boolean(of("social")?.push),
    };
  };
  await expect
    .poll(saved)
    .toEqual({ chatInbox: true, chatPush: false, socialInbox: false, socialPush: true });

  await page.reload();
  await expect(chatPush).not.toBeChecked();
  await expect(socialInbox).not.toBeChecked();
  await expect(page.getByRole("switch", { name: "チャット・通話をお知らせに残す" })).toBeChecked();

  await chatPush.click();
  await expect(chatPush).toBeChecked();
  await expect
    .poll(saved)
    .toEqual({ chatInbox: true, chatPush: true, socialInbox: false, socialPush: true });
});

test("what pushes show on the lock screen is chosen and saved", async ({ page, request, me }) => {
  await page.goto("/notifications/settings");
  const choices = page.getByRole("group", { name: "通知に内容を表示" });
  const full = choices.getByRole("radio", { name: /^すべて表示/ });
  const senderOnly = choices.getByRole("radio", { name: /^送り主だけ/ });
  const hidden = choices.getByRole("radio", { name: /^表示しない/ });
  await expect(full).toBeChecked();
  await expect(choices).toContainText("あおいさんからメッセージが届いたよ");

  await senderOnly.click();
  await expect(senderOnly).toBeChecked();
  await expect(full).not.toBeChecked();
  const saved = async () => (await preferencesOf(request, me)).pushPreview;
  await expect.poll(saved).toBe("PUSH_PREVIEW_SENDER_ONLY");

  await hidden.click();
  await expect(hidden).toBeChecked();
  await expect.poll(saved).toBe("PUSH_PREVIEW_HIDDEN");
  await page.reload();
  await expect(hidden).toBeChecked();
});

test("an account of a minor starts with only the sender shown", async ({ page, request }) => {
  const teen = await createAccount(request, "teen", {
    birthdate: `${new Date().getFullYear() - 15}-04-02`,
  });
  // Decided from the sign-up event, a moment after the account exists.
  await expect
    .poll(async () => (await preferencesOf(request, teen)).pushPreview)
    .toBe("PUSH_PREVIEW_SENDER_ONLY");
  await signInAs(page, teen);
  await page.goto("/notifications/settings");
  await expect(page.getByRole("radio", { name: /^送り主だけ/ })).toBeChecked();
});

test("quiet hours are saved with their times", async ({ page, request, me }) => {
  await page.goto("/notifications/settings");
  const quiet = page.getByRole("switch", { name: "おやすみ時間" });
  await expect(quiet).not.toBeChecked();
  await expect(page.getByText("タイムゾーン: Asia/Tokyo")).toBeVisible();

  await quiet.click();
  await page.getByLabel("はじまり").fill("23:30");
  await page.getByLabel("おわり").fill("06:15");
  await expect
    .poll(async () => (await preferencesOf(request, me)).quietHours)
    .toEqual({ enabled: true, startMinute: 1410, endMinute: 375 });

  await page.reload();
  await expect(quiet).toBeChecked();
  await expect(page.getByLabel("はじまり")).toHaveValue("23:30");
  await expect(page.getByLabel("おわり")).toHaveValue("06:15");

  // A window that starts and ends at the same time is caught before it is sent.
  await page.getByLabel("おわり").fill("23:30");
  await expect(page.getByText("はじまりとおわりは別の時間にしてね")).toBeVisible();
  await page.getByLabel("おわり").fill("07:00");
  await expect(page.getByText("はじまりとおわりは別の時間にしてね")).toHaveCount(0);
  await expect
    .poll(async () => (await preferencesOf(request, me)).quietHours)
    .toEqual({ enabled: true, startMinute: 1410, endMinute: 420 });
});

test("an account in another time zone can adopt this device's", async ({ page, request, me }) => {
  await rpc(
    request,
    "tokuly.identity.v1.AccountService/UpdateProfile",
    { timeZone: "America/New_York" },
    me,
  );

  await page.goto("/notifications/settings");
  await expect(page.getByText("タイムゾーン: America/New_York")).toBeVisible();
  await expect(page.getByText("このデバイスは Asia/Tokyo にいるみたい")).toBeVisible();
  await page.getByRole("button", { name: "合わせる" }).click();
  await expect(page.getByText("タイムゾーン: Asia/Tokyo")).toBeVisible();
  await expect(page.getByRole("button", { name: "合わせる" })).toHaveCount(0);
  await expect.poll(async () => (await preferencesOf(request, me)).timeZone).toBe("Asia/Tokyo");
});

test("push can be paused for a while, or until resumed", async ({ page, request, me }) => {
  await page.goto("/notifications/settings");
  await page.getByRole("button", { name: "1時間", exact: true }).click();
  await expect(page.getByText("停止中", { exact: true })).toBeVisible();
  await expect(page.getByText(/までプッシュ通知をお休み中だよ/)).toBeVisible();
  const pause = async () => (await preferencesOf(request, me)).pushPause ?? {};
  await expect.poll(async () => Boolean((await pause()).paused)).toBe(true);
  const until = Date.parse((await pause()).until ?? "");
  expect(Math.abs(until - (Date.now() + 3_600_000))).toBeLessThan(120_000);

  await page.reload();
  await expect(page.getByText("停止中", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "いますぐ再開する" }).click();
  await expect(page.getByRole("button", { name: "1時間", exact: true })).toBeVisible();
  await expect(page.getByText("停止中", { exact: true })).toHaveCount(0);
  await expect.poll(async () => Boolean((await pause()).paused)).toBe(false);

  await page.getByRole("button", { name: "再開するまで" }).click();
  await expect(page.getByText("再開するまでプッシュ通知をお休み中だよ")).toBeVisible();
  await expect.poll(pause).toEqual({ paused: true });
});

test("someone found by search can be muted, stays silent, and can be unmuted", async ({
  page,
  request,
  me,
}) => {
  const haru = await createAccount(request, "haru");

  await page.goto("/notifications/settings");
  await expect(page.getByText("ミュートしている人はいないよ")).toBeVisible();
  await page.getByRole("button", { name: "追加" }).click();
  const dialog = page.getByRole("dialog");
  await dialog.getByPlaceholder("なまえ または @ハンドル").fill(haru.handle);
  // The search is fuzzy and the database is shared: pick the row by its handle.
  const found = dialog.getByRole("listitem").filter({ hasText: `@${haru.handle}` });
  await found.getByRole("button", { name: "haruをミュート" }).click();
  await expect(page.getByText("haruさんをミュートしたよ")).toBeVisible();
  await expect(found.getByText("ミュート中", { exact: true })).toBeVisible();
  await dialog.getByRole("button", { name: "close" }).click();

  const muted = row(page, `@${haru.handle}`);
  await expect(muted).toBeVisible();
  const mutedHandles = async () =>
    (
      (
        await rpc<{ users?: { user: { handle: string } }[] }>(
          request,
          `${PREFERENCES}/ListMutedUsers`,
          {},
          me,
        )
      ).users ?? []
    ).map((entry) => entry.user.handle);
  expect(await mutedHandles()).toEqual([haru.handle]);

  // Nothing a muted person does arrives; someone else's request still does.
  await rpc(
    request,
    "tokuly.social.v1.FriendService/SendFriendRequest",
    { userId: me.userId },
    haru,
  );
  const sora = await friendRequestFrom(request, "sora", me);
  await expectUnread(request, me, 1);
  expect((await inboxOf(request, me)).map((entry) => entry.actor?.handle)).toEqual([sora.handle]);

  await muted.getByRole("button", { name: "haruのミュートを解除" }).click();
  await expect(page.getByText("haruさんのミュートを解除したよ")).toBeVisible();
  await expect(page.getByText("ミュートしている人はいないよ")).toBeVisible();
  await expect.poll(mutedHandles).toEqual([]);
});

// ── Web Push ────────────────────────────────────────────────────────

test("push is turned on for this browser, tested, and turned off again", async ({
  page,
  request,
  me,
}) => {
  await fakePushService(page);
  await page.goto("/notifications/settings");
  const card = page
    .locator("div")
    .filter({ has: page.getByRole("heading", { name: "このブラウザのプッシュ通知" }) })
    .last();
  await expect(card.getByText("オフ", { exact: true })).toBeVisible();
  await expect(page.getByText("プッシュ通知をオンにしたデバイスはまだないよ")).toBeVisible();
  await expect(page.getByRole("button", { name: "テスト通知を送る" })).toBeDisabled();

  await page.getByRole("button", { name: "プッシュ通知をオンにする" }).click();
  await expect(page.getByText("プッシュ通知をオンにしたよ！")).toBeVisible();
  await expect(card.getByText("オン", { exact: true })).toBeVisible();
  const device = row(page, "このブラウザ");
  await expect(device).toContainText(/Chrome \(.+\)/);
  await expect(device).toContainText("Web Push");
  expect((await devicesOf(request, me)).map((entry) => entry.deviceName)).toEqual([
    expect.stringMatching(/^Chrome \(/),
  ]);

  // Still on after a reload: the registration belongs to this browser and session.
  await page.reload();
  await expect(card.getByText("オン", { exact: true })).toBeVisible();

  await page.getByRole("button", { name: "テスト通知を送る" }).click();
  await expect(page.getByText("テスト通知を1台に送ったよ！")).toBeVisible();

  await page.getByRole("button", { name: "このブラウザでオフにする" }).click();
  await expect(page.getByText("プッシュ通知をオフにしたよ")).toBeVisible();
  await expect(card.getByText("オフ", { exact: true })).toBeVisible();
  await expect(page.getByText("プッシュ通知をオンにしたデバイスはまだないよ")).toBeVisible();
  await expect.poll(() => devicesOf(request, me)).toEqual([]);
});

test("a browser whose push service refuses is told so and stays off", async ({ page, me: _me }) => {
  // Headless Chrome as it is: permission granted, but no push service to subscribe with.
  await page.goto("/notifications/settings");
  await page.getByRole("button", { name: "プッシュ通知をオンにする" }).click();
  await expect(page.getByText("このブラウザでプッシュ通知を登録できなかったよ")).toBeVisible();
  await expect(page.getByRole("button", { name: "プッシュ通知をオンにする" })).toBeVisible();
});

test("a browser without Web Push is told so, and is never asked", async ({ page, request, me }) => {
  await page.addInitScript(() => {
    delete (window as { PushManager?: unknown }).PushManager;
  });
  await friendRequestFrom(request, "haru", me);
  await expectUnread(request, me, 1);

  await page.goto("/notifications/settings");
  await expect(page.getByText("非対応", { exact: true })).toBeVisible();
  await expect(page.getByText("このブラウザはプッシュ通知に対応していないよ")).toBeVisible();
  await expect(page.getByRole("button", { name: "プッシュ通知をオンにする" })).toHaveCount(0);

  await page.goto("/notifications");
  await expect(row(page, "haruさんから")).toBeVisible();
  await expect(page.getByText("プッシュ通知をオンにしよう！")).toHaveCount(0);
});

test("blocked notifications are explained instead of asked for again", async ({
  page,
  me: _me,
}) => {
  await page.addInitScript(() =>
    Object.defineProperty(Notification, "permission", { configurable: true, get: () => "denied" }),
  );
  await page.goto("/notifications/settings");
  await expect(page.getByText("ブロック中", { exact: true })).toBeVisible();
  await expect(page.getByText(/通知がブラウザでブロックされているよ/)).toBeVisible();
  await expect(page.getByRole("button", { name: "プッシュ通知をオンにする" })).toHaveCount(0);
});

test("a server without Web Push is shown as unavailable", async ({ page, me: _me }) => {
  // An empty GetPushConfig response: no VAPID key, no channels.
  await page.route("**/api/tokuly.notify.v1.PushService/GetPushConfig", (route) =>
    route.fulfill({ status: 200, contentType: "application/proto", body: Buffer.alloc(0) }),
  );
  await page.goto("/notifications/settings");
  await expect(page.getByText("利用不可", { exact: true })).toBeVisible();
  await expect(page.getByText("このサーバーではプッシュ通知がまだ使えないよ")).toBeVisible();
  await expect(page.getByRole("button", { name: "プッシュ通知をオンにする" })).toHaveCount(0);
});

test("the inbox suggests push once there is something in it, and can be told 'later'", async ({
  page,
  request,
  me,
}) => {
  await page.goto("/notifications");
  await expect(page.getByText("お知らせはまだないよ")).toBeVisible();
  // Never on an empty first visit.
  await expect(page.getByText("プッシュ通知をオンにしよう！")).toHaveCount(0);

  await friendRequestFrom(request, "haru", me);
  await expectUnread(request, me, 1);
  await page.reload();
  await expect(page.getByText("プッシュ通知をオンにしよう！")).toBeVisible();
  await page.getByRole("button", { name: "あとで" }).click();
  await expect(page.getByText("プッシュ通知をオンにしよう！")).toHaveCount(0);

  await page.reload();
  await expect(row(page, "haruさんから")).toBeVisible();
  await expect(page.getByText("プッシュ通知をオンにしよう！")).toHaveCount(0);
});

test("push can be turned on straight from the inbox", async ({ page, request, me }) => {
  await fakePushService(page);
  await friendRequestFrom(request, "haru", me);
  await expectUnread(request, me, 1);

  await page.goto("/notifications");
  const prompt = page.getByRole("region", { name: "プッシュ通知をオンにしよう！" });
  await prompt.getByRole("button", { name: "オンにする" }).click();
  await expect(page.getByText("プッシュ通知をオンにしたよ！")).toBeVisible();
  await expect(prompt).toHaveCount(0);
  await expect.poll(async () => (await devicesOf(request, me)).length).toBe(1);
});

// ── Service worker ──────────────────────────────────────────────────

test("the service worker shows a pushed notification, and a tap on it opens the entry in the app", async ({
  page,
  request,
  me,
  context,
}) => {
  const haru = await friendRequestFrom(request, "haru", me);
  await expectUnread(request, me, 1);
  const [entry] = await inboxOf(request, me);
  if (!entry) throw new Error("no inbox entry");

  await page.goto("/me");
  await expect(page.getByRole("navigation", { name: "main" })).toBeVisible();
  // The app registers the worker by itself. A push only ever reaches an activated worker.
  await page.evaluate(() => navigator.serviceWorker.ready.then(() => undefined));
  const worker = context.serviceWorkers()[0] ?? (await context.waitForEvent("serviceworker"));
  await expect
    .poll(() => worker.evaluate(`self.registration.active?.state ?? "none"`), { timeout: 20_000 })
    .toBe("activated");

  // The payload the server sends for this entry (docs/events/notify.md), delivered as a push event.
  const payload = {
    web_push: 8030,
    notification: {
      title: entry.title,
      body: entry.body,
      lang: "ja",
      navigate: `http://tokuly.example${entry.path}`,
      tag: `e2e:${entry.id}`,
      timestamp: Date.now(),
      data: { id: entry.id, kind: entry.kind, category: "social", path: entry.path },
    },
    app_badge: 1,
  };
  // Delivered as the browser would, and waited for: the handler's work (showing the notification)
  // is what the event's `waitUntil` receives, so a failure there fails here, with its reason.
  await worker.evaluate(`new Promise((resolve, reject) => {
    const event = new PushEvent("push", { data: ${JSON.stringify(JSON.stringify(payload))} });
    const waitUntil = event.waitUntil.bind(event);
    event.waitUntil = (work) => {
      Promise.resolve(work).then(resolve, (error) => reject(String(error)));
      try { waitUntil(work); } catch {}
    };
    self.dispatchEvent(event);
  })`);
  const shown = () =>
    page.evaluate(async () =>
      (await (await navigator.serviceWorker.ready).getNotifications()).map((notification) => ({
        title: notification.title,
        body: notification.body,
        tag: notification.tag,
      })),
    );
  await expect
    .poll(shown)
    .toEqual([{ title: entry.title, body: entry.body, tag: `e2e:${entry.id}` }]);

  // The tap: the worker hands it to the open app, which goes there without reloading and marks the entry read.
  await worker.evaluate(
    `self.registration.getNotifications().then(([notification]) => { self.dispatchEvent(new NotificationEvent("notificationclick", { notification })); })`,
  );
  await expect(page).toHaveURL(new RegExp(`/u/${haru.handle}$`));
  await expectUnread(request, me, 0);
  await expect.poll(shown).toEqual([]);
});

test("a tap on a system notification while the app was closed lands on its target", async ({
  page,
  request,
  me,
}) => {
  const haru = await friendRequestFrom(request, "haru", me);
  await expectUnread(request, me, 1);
  const [entry] = await inboxOf(request, me);
  if (!entry) throw new Error("no inbox entry");

  // Where the service worker opens a window when no page of the app exists.
  await page.goto(`/notifications/open?path=${encodeURIComponent(entry.path)}&id=${entry.id}`);
  await expect(page).toHaveURL(new RegExp(`/u/${haru.handle}$`));
  await expectUnread(request, me, 0);

  // The test push points at the settings under the path the server knows them by.
  await page.goto(`/notifications/open?path=${encodeURIComponent("/settings/notifications")}`);
  await expect(page).toHaveURL(/\/notifications\/settings$/);
  await expect(page.getByRole("heading", { name: "通知設定" })).toBeVisible();
});
