import zlib from "node:zlib";
import type { APIRequestContext, Browser, Locator, Page, TestInfo } from "@playwright/test";
import { type Account, expect as baseExpect, befriend, createAccount, rpc, signInAs, test } from "./support";

// These tests share a machine with many other builds and browsers: a cold page
// or a busy API can take a while to answer. Waiting longer costs nothing when
// things are quick.
const expect = baseExpect.configure({ timeout: 20_000 });
test.describe.configure({ timeout: 120_000 });

// Calls, voice messages and POV use the camera and the microphone. Chrome's fake devices stand in for
// them (a test pattern, a beep) and no permission prompt appears. Launch options replace the ones of
// the configuration as a whole, so its flags are repeated here.
test.use({
  launchOptions: {
    args: ["--use-angle=swiftshader", "--enable-unsafe-swiftshader", "--ignore-gpu-blocklist", "--enable-webgl", "--disable-dev-shm-usage", "--use-fake-device-for-media-stream", "--use-fake-ui-for-media-stream"],
  },
});

const CONVERSATIONS = "tokuly.chat.v1.ConversationService";
const MESSAGES = "tokuly.chat.v1.MessageService";
const POV = "tokuly.chat.v1.PovService";
const CALLS = "tokuly.chat.v1.CallService";
const MEDIA = "tokuly.media.v1.MediaService";
const STAMPS = "tokuly.stamp.v1.StampLibraryService";

// ───────────────────────────── Arranging and checking through the API ─────────────────────────────

/** A message as the JSON API returns it (unset fields are left out). */
interface ApiMessage {
  id: string;
  kind: string;
  text?: string;
  deleted?: boolean;
  editedAt?: string;
  stamp?: { name: string };
  attachments?: { kind: string }[];
  reactions?: { emoji: string; count: number }[];
  replyTo?: { messageId: string };
  call?: { state: string; endReason?: string };
  povRequest?: { id: string; state: string };
}

/** The direct conversation between `as` and `other` (created when they never talked). */
async function directWith(request: APIRequestContext, as: Account, other: Account): Promise<string> {
  const { conversation } = await rpc<{ conversation: { id: string } }>(request, `${CONVERSATIONS}/OpenDirect`, { userId: other.userId }, as);
  return conversation.id;
}

async function sendText(request: APIRequestContext, as: Account, conversationId: string, text: string): Promise<ApiMessage> {
  const { message } = await rpc<{ message: ApiMessage }>(request, `${MESSAGES}/SendMessage`, { conversationId, clientId: crypto.randomUUID(), text }, as);
  return message;
}

async function messagesOf(request: APIRequestContext, as: Account, conversationId: string): Promise<ApiMessage[]> {
  return (await rpc<{ messages?: ApiMessage[] }>(request, `${MESSAGES}/ListMessages`, { conversationId, limit: 100 }, as)).messages ?? [];
}

/** Waits until `as` can see a message matching `matches` in the conversation, and returns it. */
async function expectMessage(request: APIRequestContext, as: Account, conversationId: string, matches: (message: ApiMessage) => boolean): Promise<ApiMessage> {
  let found: ApiMessage | undefined;
  await expect(async () => {
    found = (await messagesOf(request, as, conversationId)).find(matches);
    expect(found, "the message on the server").toBeTruthy();
  }).toPass({ timeout: 20_000 });
  return found as ApiMessage;
}

/** Two accounts that are friends, with their direct conversation: `me` (signed in in the browser) and "aoi". */
async function withFriend(request: APIRequestContext, me: Account, name = "aoi"): Promise<{ friend: Account; conversationId: string }> {
  const friend = await createAccount(request, name);
  await befriend(request, me, friend);
  return { friend, conversationId: await directWith(request, friend, me) };
}

/** A PNG picture: a coloured disc on white. */
function picture(name: string, color: [number, number, number] = [230, 40, 60], size = 160): { name: string; mimeType: string; buffer: Buffer } {
  const row = size * 4 + 1;
  const raw = Buffer.alloc(row * size);
  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      const inside = Math.hypot(x - size / 2, y - size / 2) < size * 0.3;
      raw.set(inside ? [...color, 255] : [255, 255, 255, 255], y * row + 1 + x * 4);
    }
  }
  const chunk = (type: string, data: Buffer) => {
    const body = Buffer.concat([Buffer.from(type), data]);
    const out = Buffer.alloc(body.length + 8);
    out.writeUInt32BE(data.length, 0);
    body.copy(out, 4);
    out.writeUInt32BE(zlib.crc32(body) >>> 0, body.length + 4);
    return out;
  };
  const header = Buffer.alloc(13);
  header.writeUInt32BE(size, 0);
  header.writeUInt32BE(size, 4);
  header.set([8, 6, 0, 0, 0], 8);
  const signature = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
  return { name, mimeType: "image/png", buffer: Buffer.concat([signature, chunk("IHDR", header), chunk("IDAT", zlib.deflateSync(raw)), chunk("IEND", Buffer.alloc(0))]) };
}

/** Gives `as` a personal stamp called `name`, ready to send. */
async function personalStamp(request: APIRequestContext, as: Account, name: string): Promise<void> {
  const file = picture("stamp.png", [60, 120, 240], 240);
  const created = await rpc<{ assetId: string; uploadId: string; putUrl: string; putHeaders?: Record<string, string> }>(
    request,
    `${MEDIA}/CreateUpload`,
    { purpose: "ASSET_PURPOSE_STAMP", fileName: file.name, contentType: file.mimeType, sizeBytes: String(file.buffer.length) },
    as,
  );
  const put = await request.put(created.putUrl, { data: file.buffer, headers: created.putHeaders ?? {} });
  expect(put.ok(), "stamp upload").toBeTruthy();
  await rpc(request, `${MEDIA}/CompleteUpload`, { uploadId: created.uploadId }, as);
  await expect(async () => {
    const { asset } = await rpc<{ asset: { status: string } }>(request, `${MEDIA}/GetAsset`, { assetId: created.assetId }, as);
    expect(asset.status).toBe("ASSET_STATUS_READY");
  }).toPass({ timeout: 30_000 });
  await rpc(request, `${STAMPS}/CreatePersonalStamp`, { assetId: created.assetId, name }, as);
}

// ───────────────────────────── Driving the UI ─────────────────────────────

/** Opens the conversation with `conversationId` and waits until it can be written in. */
async function openConversation(page: Page, conversationId: string): Promise<void> {
  await page.goto(`/chat/${conversationId}`);
  await expect(page.getByTestId("composer")).toBeVisible();
}

const composer = (page: Page) => page.getByRole("textbox", { name: "メッセージを入力" });

async function send(page: Page, text: string): Promise<void> {
  await composer(page).fill(text);
  await page.getByRole("button", { name: "送信", exact: true }).click();
}

/** The bubble of the message saying `text`. */
const bubble = (page: Page, text: string | RegExp): Locator => page.getByTestId("message").filter({ hasText: text });

/** Opens the actions of a message (right-click; a long press on touch screens). */
async function openActions(page: Page, text: string | RegExp): Promise<void> {
  await bubble(page, text).getByText(text).first().click({ button: "right" });
  await expect(page.getByRole("menu")).toBeVisible();
}

/** The unread counter on the chat entry of the navigation (the rail on wide screens, the tab bar on phones). */
const chatBadge = (page: Page): Locator => page.locator('nav[aria-label="main"] a[href="/chat"]:visible');

/** A second person's browser, signed in, at the size of the running project. */
async function browserOf(browser: Browser, testInfo: TestInfo, account: Account): Promise<Page> {
  const context = await browser.newContext({ baseURL: testInfo.project.use.baseURL, locale: "ja-JP", timezoneId: "Asia/Tokyo", viewport: testInfo.project.use.viewport, hasTouch: testInfo.project.use.hasTouch });
  const page = await context.newPage();
  await signInAs(page, account);
  return page;
}

// ───────────────────────────── Messages ─────────────────────────────

test("a profile's message link opens the direct conversation, and text goes both ways live", async ({ page, request, me }) => {
  const { friend, conversationId } = await withFriend(request, me);

  // `?user=<handle>` is how other screens link to a person's chat.
  await page.goto(`/chat?user=${friend.handle}`);
  await expect(page).toHaveURL(new RegExp(`/chat/${conversationId}$`));
  await expect(page.getByTestId("conversation-title")).toHaveText("aoi");

  await send(page, "やっほー！");
  await expect(bubble(page, "やっほー！")).toBeVisible();
  await expectMessage(request, friend, conversationId, (message) => message.text === "やっほー！");

  // What the friend writes shows up without reloading.
  await sendText(request, friend, conversationId, "おつかれ〜");
  await expect(bubble(page, "おつかれ〜")).toBeVisible();
  await expect(bubble(page, "おつかれ〜")).not.toHaveAttribute("data-own");

  // The message is in the list preview too.
  if ((page.viewportSize()?.width ?? 0) < 768) await page.getByRole("link", { name: "もどる" }).click();
  await expect(page.getByTestId("conversation-row").filter({ hasText: "aoi" })).toContainText("おつかれ〜");
});

test("a message sent here arrives live in the other person's open conversation, and they see it was read", async ({ page, request, me, browser }, testInfo) => {
  const { friend, conversationId } = await withFriend(request, me);
  const theirs = await browserOf(browser, testInfo, friend);
  await openConversation(theirs, conversationId);
  await openConversation(page, conversationId);

  // Both have it open: each sees the other is here.
  await expect(page.getByTestId("conversation-subtitle")).toHaveText("いまトークを見てるよ");

  await send(page, "今ひま？");
  await expect(bubble(theirs, "今ひま？")).toBeVisible();
  // They are looking at it, so it is read at once.
  await expect(bubble(page, "今ひま？").getByTestId("read-mark")).toHaveText("既読");

  // Typing shows on the other side while it happens.
  await composer(theirs).pressSequentially("ひまだよ");
  await expect(page.getByTestId("typing-indicator")).toBeVisible();
  await theirs.getByRole("button", { name: "送信", exact: true }).click();
  await expect(bubble(page, "ひまだよ")).toBeVisible();
  await expect(page.getByTestId("typing-indicator")).toHaveCount(0);
  await theirs.context().close();
});

test("an unread message shows on the navigation and clears when the conversation is opened", async ({ page, request, me }) => {
  const { friend, conversationId } = await withFriend(request, me);
  await page.goto("/friends");
  await expect(page.getByRole("heading", { name: "ともだち" })).toBeVisible();
  await expect(chatBadge(page)).not.toContainText(/\d/);

  await sendText(request, friend, conversationId, "みてみて！");
  await sendText(request, friend, conversationId, "すごいの見つけた");
  await expect(chatBadge(page)).toContainText("2");

  await page.goto("/chat");
  const row = page.getByTestId("conversation-row").filter({ hasText: "aoi" });
  await expect(row).toContainText("すごいの見つけた");
  await expect(row).toContainText("2");

  await row.click();
  // The conversation opens at "unread from here", and reading it clears the counters.
  await expect(page.getByTestId("unread-divider")).toBeVisible();
  await expect(bubble(page, "すごいの見つけた")).toBeVisible();
  await expect(chatBadge(page)).not.toContainText(/\d/);
  const unread = await rpc<{ unread?: { badge?: number } }>(request, `${CONVERSATIONS}/GetUnreadCounts`, {}, me);
  expect(unread.unread?.badge ?? 0).toBe(0);
});

test("a stamp is picked and sent", async ({ page, request, me }) => {
  test.setTimeout(150_000);
  const { friend, conversationId } = await withFriend(request, me);
  await personalStamp(request, me, "あおまる");
  await openConversation(page, conversationId);

  await page.getByRole("button", { name: "スタンプ", exact: true }).click();
  const picker = page.getByTestId("stamp-picker");
  await picker.getByRole("tab", { name: "じぶんのスタンプ" }).click();
  await picker.getByRole("button", { name: "あおまる" }).click();

  await expect(page.getByTestId("stamp-message")).toBeVisible();
  const sent = await expectMessage(request, friend, conversationId, (message) => message.kind === "MESSAGE_KIND_STAMP");
  expect(sent.stamp?.name).toBe("あおまる");
});

test("a photo is attached, sent, and opens full screen", async ({ page, request, me }) => {
  test.setTimeout(150_000);
  const { friend, conversationId } = await withFriend(request, me);
  await openConversation(page, conversationId);

  await page.getByTestId("photo-input").setInputFiles(picture("sunset.png"));
  await expect(page.getByTestId("attachment-chip")).toHaveCount(1);
  await send(page, "きれいだった！");

  // It is in the conversation at once (with its caption), and on the server shortly after.
  const message = bubble(page, "きれいだった！");
  await expect(message).toBeVisible();
  await expect(message).not.toHaveAttribute("data-pending", /.+/, { timeout: 60_000 });
  const sent = await expectMessage(request, friend, conversationId, (candidate) => candidate.kind === "MESSAGE_KIND_MEDIA");
  expect(sent.attachments?.[0]?.kind).toBe("ASSET_KIND_IMAGE");
  expect(sent.text).toBe("きれいだった！");

  await message.getByRole("button", { name: "写真をひらく" }).click();
  await expect(page.getByTestId("lightbox")).toBeVisible();
  await page.getByTestId("lightbox").getByRole("button", { name: "閉じる" }).click();
  await expect(page.getByTestId("lightbox")).toHaveCount(0);
});

test("a message is answered, reacted to, edited and unsent", async ({ page, request, me }) => {
  const { friend, conversationId } = await withFriend(request, me);
  const question = await sendText(request, friend, conversationId, "明日なんじ集合？");
  await openConversation(page, conversationId);

  // Reply: the answer carries the quoted message.
  await openActions(page, "明日なんじ集合？");
  await page.getByRole("menuitem", { name: "返信" }).click();
  await expect(page.getByTestId("reply-bar")).toContainText("明日なんじ集合？");
  await send(page, "10時でどう？");
  await expect(bubble(page, "10時でどう？")).toContainText("明日なんじ集合？");
  const answer = await expectMessage(request, friend, conversationId, (message) => message.text === "10時でどう？");
  expect(answer.replyTo?.messageId).toBe(question.id);

  // React: one tap on a quick reaction, and it can be taken back by tapping the chip.
  await openActions(page, "明日なんじ集合？");
  await page.getByRole("menuitem", { name: "❤️ でリアクション" }).click();
  const chip = bubble(page, "明日なんじ集合？").getByTestId("reactions").getByRole("button", { name: /❤️/ });
  await expect(chip).toHaveAttribute("aria-pressed", "true");
  await expectMessage(request, friend, conversationId, (message) => message.id === question.id && message.reactions?.[0]?.emoji === "❤️");
  // A reaction from the other side joins in live.
  await rpc(request, `${MESSAGES}/AddReaction`, { messageId: answer.id, emoji: "👍" }, friend);
  await expect(bubble(page, "10時でどう？").getByTestId("reactions")).toContainText("👍");

  // Edit.
  await openActions(page, "10時でどう？");
  await page.getByRole("menuitem", { name: "編集" }).click();
  await expect(page.getByTestId("editing-bar")).toBeVisible();
  await page.getByRole("textbox", { name: "メッセージを入力" }).fill("やっぱ11時で！");
  await page.getByRole("button", { name: "保存", exact: true }).click();
  await expect(bubble(page, "やっぱ11時で！")).toContainText("編集済み");
  await expectMessage(request, friend, conversationId, (message) => message.id === answer.id && message.text === "やっぱ11時で！");

  // Unsend: a tombstone stays for everyone.
  await openActions(page, "やっぱ11時で！");
  await page.getByRole("menuitem", { name: "送信取消" }).click();
  await page.getByRole("dialog").getByRole("button", { name: "送信取消" }).click();
  await expect(page.getByTestId("message").filter({ hasText: "メッセージの送信を取り消しました" })).toBeVisible();
  await expect(bubble(page, "やっぱ11時で！")).toHaveCount(0);
  await expectMessage(request, friend, conversationId, (message) => message.id === answer.id && message.deleted === true);
});

test("a message that could not be sent stays with a way to try again", async ({ page, request, me }) => {
  const { conversationId } = await withFriend(request, me);
  await openConversation(page, conversationId);

  // The server is out of reach for the first attempt.
  let failing = true;
  await page.route("**/api/tokuly.chat.v1.MessageService/SendMessage", (route) => (failing ? route.abort("connectionfailed") : route.continue()));
  await send(page, "とどけ〜");
  const message = bubble(page, "とどけ〜");
  await expect(message).toHaveAttribute("data-pending", "failed");

  failing = false;
  await message.getByRole("button", { name: "もう一度送る" }).click();
  await expect(message).not.toHaveAttribute("data-pending", /.+/);
  await expect(page.getByTestId("message").filter({ hasText: "とどけ〜" })).toHaveCount(1);
});

test("a pinned message is kept at the top and found again; search finds old messages", async ({ page, request, me }) => {
  const { friend, conversationId } = await withFriend(request, me);
  await sendText(request, friend, conversationId, "集合は駅の東口ね");
  for (let index = 1; index <= 30; index++) await sendText(request, friend, conversationId, `つぶやき ${index}`);
  await openConversation(page, conversationId);
  // With this much unread, the conversation opens where the reading left off; one tap goes to the newest.
  await expect(page.getByTestId("unread-divider")).toBeVisible();
  await expect(bubble(page, "集合は駅の東口ね")).toBeVisible();
  await page.getByTestId("jump-to-latest").click();
  await expect(bubble(page, "つぶやき 30")).toBeVisible();

  // Search finds the old message and jumps to it.
  await page.getByRole("button", { name: "トークのメニュー" }).click();
  await page.getByRole("menuitem", { name: "さがす" }).click();
  await page.getByRole("textbox", { name: "メッセージをさがす" }).fill("東口");
  await page.getByTestId("message-result").filter({ hasText: "集合は駅の東口ね" }).click();
  await expect(bubble(page, "集合は駅の東口ね")).toBeVisible();

  // Pin it: a bar under the header shows it from now on.
  await openActions(page, "集合は駅の東口ね");
  await page.getByRole("menuitem", { name: "ピン留め", exact: true }).click();
  await expect(page.getByTestId("pinned-bar")).toContainText("集合は駅の東口ね");
  // The conversation notes who pinned it, down at the newest messages.
  await page.getByTestId("jump-to-latest").click();
  await expect(page.getByTestId("system-message").filter({ hasText: "ピン留めしました" })).toBeVisible();
});

// ───────────────────────────── Conversations ─────────────────────────────

test("a group is created with friends and renamed", async ({ page, request, me }) => {
  const aoi = await createAccount(request, "aoi");
  const ren = await createAccount(request, "ren");
  await befriend(request, me, aoi);
  await befriend(request, me, ren);
  await page.goto("/chat");

  await page.getByRole("button", { name: "新しいチャット" }).click();
  const dialog = page.getByRole("dialog");
  await dialog.getByRole("button", { name: /グループをつくる/ }).click();
  await dialog.getByLabel("グループ名").fill("放課後メンバー");
  await dialog.getByRole("button", { name: "aoi" }).click();
  await dialog.getByRole("button", { name: "ren" }).click();
  await dialog.getByRole("button", { name: "グループをつくる", exact: true }).click();

  // The new group opens, and its creation is noted in the conversation.
  await expect(page.getByTestId("conversation-title")).toHaveText("放課後メンバー");
  await expect(page.getByTestId("system-message").first()).toContainText("放課後メンバー");
  await send(page, "よろしく〜！");
  await expect(bubble(page, "よろしく〜！")).toBeVisible();

  // Rename it from the details.
  await page.getByRole("button", { name: "放課後メンバーのトーク情報" }).click();
  await expect(page.getByTestId("member-row")).toHaveCount(3);
  await page.getByRole("button", { name: "グループ名を変更" }).click();
  await page.getByRole("textbox", { name: "グループ名" }).fill("カラオケ部");
  await page.getByRole("textbox", { name: "グループ名" }).press("Enter");
  await expect(page.getByTestId("conversation-title")).toHaveText("カラオケ部");
  await expect(page.getByTestId("system-message").filter({ hasText: "カラオケ部" })).toBeVisible();

  // The others have the group, under its new name.
  const theirs = await rpc<{ conversations?: { name: string }[] }>(request, `${CONVERSATIONS}/ListConversations`, {}, aoi);
  expect(theirs.conversations?.map((conversation) => conversation.name)).toContain("カラオケ部");
});

test("a message from someone who is not a friend waits as a request until it is accepted", async ({ page, request, me }) => {
  // Requests reach people who accept messages from everyone.
  const { settings } = await rpc<{ settings: { privacy: object } }>(request, "tokuly.identity.v1.AccountService/GetSettings", {}, me);
  await rpc(request, "tokuly.identity.v1.AccountService/UpdateSettings", { privacy: { ...settings.privacy, directMessagesFrom: "AUDIENCE_EVERYONE" } }, me);
  const stranger = await createAccount(request, "kai");
  const conversationId = await directWith(request, stranger, me);
  await sendText(request, stranger, conversationId, "はじめまして！");

  await page.goto("/chat");
  // Not in the inbox, and not counted as unread there: it waits under "requests".
  await expect(page.getByTestId("conversation-row")).toHaveCount(0);
  const requests = page.getByRole("tab", { name: /リクエスト/ });
  await expect(requests).toContainText("1");
  await requests.click();
  await page.getByTestId("conversation-row").filter({ hasText: "kai" }).click();

  await expect(bubble(page, "はじめまして！")).toBeVisible();
  await expect(page.getByTestId("composer")).toHaveCount(0);
  await page.getByTestId("request-bar").getByRole("button", { name: "承認" }).click();

  // Accepted: it can be answered, and it is an ordinary conversation from now on.
  await send(page, "よろしくね！");
  await expect(bubble(page, "よろしくね！")).toBeVisible();
  await expectMessage(request, stranger, conversationId, (message) => message.text === "よろしくね！");
  const inbox = await rpc<{ conversations?: { id: string }[] }>(request, `${CONVERSATIONS}/ListConversations`, {}, me);
  expect(inbox.conversations?.map((conversation) => conversation.id)).toContain(conversationId);
});

test("a conversation is pinned, muted and hidden from its menu", async ({ page, request, me }) => {
  const { friend, conversationId } = await withFriend(request, me);
  const { friend: other, conversationId: otherId } = await withFriend(request, me, "ren");
  await sendText(request, friend, conversationId, "さきに送ったよ");
  await sendText(request, other, otherId, "あとから送ったよ");
  await page.goto("/chat");
  const rows = page.getByTestId("conversation-row");
  await expect(rows.first()).toContainText("ren");

  // Pinned conversations come first, whatever is newer.
  await rows.filter({ hasText: "aoi" }).click({ button: "right" });
  await page.getByRole("menuitem", { name: "ピン留めする" }).click();
  await expect(rows.first()).toContainText("aoi");

  // Muted for an hour.
  await rows.filter({ hasText: "ren" }).click({ button: "right" });
  await page.getByRole("menuitem", { name: "通知をオフ" }).click();
  await page.getByRole("dialog").getByRole("button", { name: "1時間" }).click();
  await expect(rows.filter({ hasText: "ren" }).getByLabel("通知オフ")).toBeVisible();

  // Hidden until the next message.
  await rows.filter({ hasText: "ren" }).click({ button: "right" });
  await page.getByRole("menuitem", { name: "非表示にする" }).click();
  await page.getByRole("dialog").getByRole("button", { name: "非表示にする" }).click();
  await expect(rows).toHaveCount(1);
  await sendText(request, other, otherId, "ただいま！");
  await expect(rows.filter({ hasText: "ren" })).toContainText("ただいま！");
});

// ───────────────────────────── POV ─────────────────────────────

test("a POV request arrives, counts down, and can be declined", async ({ page, request, me }) => {
  const { friend, conversationId } = await withFriend(request, me);
  await openConversation(page, conversationId);

  await rpc(request, `${POV}/RequestPov`, { conversationId, note: "今どこ？" }, friend);
  const card = page.getByTestId("pov-request");
  await expect(card).toContainText("aoiがPOVをリクエスト！");
  await expect(card).toContainText("今どこ？");
  await expect(card).toContainText(/あと 1:5\d:\d\d/);
  await expect(page.getByTestId("pov-banner")).toBeVisible();

  await card.getByRole("button", { name: "ごめんね" }).click();
  await expect(card).toContainText("今回はパスしました");
  await expect(page.getByTestId("pov-banner")).toHaveCount(0);
  const asked = await expectMessage(request, friend, conversationId, (message) => message.kind === "MESSAGE_KIND_POV_REQUEST");
  expect(asked.povRequest?.state).toBe("POV_STATE_DECLINED");
});

test("a POV is asked for from the composer and withdrawn again", async ({ page, request, me }) => {
  const { friend, conversationId } = await withFriend(request, me);
  await openConversation(page, conversationId);

  await page.getByRole("button", { name: "追加する" }).click();
  await page.getByRole("menuitem", { name: "POVをリクエスト" }).click();
  const dialog = page.getByRole("dialog");
  await dialog.getByRole("button", { name: "なにしてる？" }).click();
  await dialog.getByRole("button", { name: "リクエストする" }).click();

  const card = page.getByTestId("pov-request");
  await expect(card).toContainText("aoiにPOVをリクエスト中！");
  await expect(card).toContainText("なにしてる？");
  const asked = await expectMessage(request, friend, conversationId, (message) => message.kind === "MESSAGE_KIND_POV_REQUEST");
  expect(asked.povRequest?.state).toBe("POV_STATE_OPEN");

  await card.getByRole("button", { name: "リクエストを取り消す" }).click();
  await expect(card).toContainText("リクエストは取り消されました");
});

// ───────────────────────────── Calls and the microphone ─────────────────────────────

test("a call rings on the other person's screen wherever they are, and declining ends it", async ({ page, request, me, browser }, testInfo) => {
  const { friend, conversationId } = await withFriend(request, me);
  const theirs = await browserOf(browser, testInfo, friend);
  await theirs.goto("/friends");
  await expect(theirs.getByRole("heading", { name: "ともだち" })).toBeVisible();
  await openConversation(page, conversationId);

  await page.getByRole("button", { name: "音声通話" }).click();
  await expect(page.getByTestId("call-screen")).toBeVisible();
  await expect(page.getByTestId("call-status")).toHaveText("呼び出し中…");

  const ringing = theirs.getByTestId("incoming-call");
  await expect(ringing).toContainText("me");
  await expect(ringing).toContainText("音声通話がきてるよ！");
  await ringing.getByRole("button", { name: "拒否する" }).click();
  await expect(ringing).toHaveCount(0);

  // The caller's screen closes, and the conversation remembers the call.
  await expect(page.getByTestId("call-screen")).toHaveCount(0);
  await expect(page.getByTestId("call-message")).toBeVisible();
  const call = await expectMessage(request, me, conversationId, (message) => message.kind === "MESSAGE_KIND_CALL" && message.call?.state === "CALL_STATE_ENDED");
  expect(call.call?.endReason).toBe("CALL_END_REASON_DECLINED");
  await theirs.context().close();
});

test("an answered call connects both sides and ends for both when one hangs up", async ({ page, request, me, browser }, testInfo) => {
  const { friend, conversationId } = await withFriend(request, me);
  const theirs = await browserOf(browser, testInfo, friend);
  await theirs.goto("/chat");
  await openConversation(page, conversationId);

  await page.getByRole("button", { name: "ビデオ通話" }).click();
  await theirs.getByTestId("incoming-call").getByRole("button", { name: "応答する" }).click();

  // Both are in the call: the timer runs and each has a tile for the other.
  await expect(page.getByTestId("call-status")).toHaveText(/^\d+:\d\d$/, { timeout: 40_000 });
  await expect(theirs.getByTestId("call-status")).toHaveText(/^\d+:\d\d$/, { timeout: 40_000 });
  await expect(page.getByTestId("call-tile")).toHaveCount(2);
  await expect(theirs.getByTestId("call-tile")).toHaveCount(2);

  // Muting is a tap, and the call can be put aside while using the app.
  await page.getByRole("button", { name: "マイクをオフ" }).click();
  await expect(page.getByRole("button", { name: "マイクをオン" })).toBeVisible();
  await page.getByRole("button", { name: "小さくする" }).click();
  await expect(page.getByTestId("call-pill")).toBeVisible();
  await page.getByRole("button", { name: "通話画面にもどる" }).click();

  await page.getByRole("button", { name: "通話を終了" }).click();
  await expect(page.getByTestId("call-screen")).toHaveCount(0);
  await expect(theirs.getByTestId("call-screen")).toHaveCount(0);
  const call = await expectMessage(request, me, conversationId, (message) => message.kind === "MESSAGE_KIND_CALL" && message.call?.state === "CALL_STATE_ENDED");
  expect(call.call?.endReason).toBe("CALL_END_REASON_COMPLETED");

  // It is in the call history of both.
  await page.goto("/chat/calls");
  await expect(page.getByTestId("call-row").filter({ hasText: "aoi" })).toBeVisible();
  const history = await rpc<{ calls?: { id: string }[] }>(request, `${CALLS}/ListCalls`, {}, friend);
  expect(history.calls).toHaveLength(1);
  await theirs.context().close();
});

test("a voice message is recorded and sent", async ({ page, request, me }) => {
  test.setTimeout(150_000);
  const { friend, conversationId } = await withFriend(request, me);
  await openConversation(page, conversationId);

  await page.getByRole("button", { name: "ボイスメッセージを録音" }).click();
  await expect(page.getByTestId("voice-recording")).toBeVisible();
  await page.waitForTimeout(1600);
  await page.getByRole("button", { name: "ボイスメッセージを送信" }).click();

  await expect(page.getByTestId("voice-message")).toBeVisible();
  const sent = await expectMessage(request, friend, conversationId, (message) => message.kind === "MESSAGE_KIND_MEDIA");
  expect(sent.attachments?.[0]?.kind).toBe("ASSET_KIND_AUDIO");
});
