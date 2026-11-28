import type { APIRequestContext, Browser, Page, TestInfo } from "@playwright/test";
import { holdAnswers } from "./answers";
import {
  type Account,
  expect as baseExpect,
  befriend,
  createAccount,
  RpcError,
  rpc,
  signInAs,
  test,
} from "./support";

// Going live, archives and the media server take a while on a busy machine.
const expect = baseExpect.configure({ timeout: 20_000 });
test.describe.configure({ timeout: 180_000 });

// Publishing from the browser uses the camera and the microphone. Chrome's fake devices stand in for
// them (a test pattern, a beep) and no permission prompt appears. Launch options replace the ones of
// the configuration as a whole, so its flags are repeated here.
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
    ],
  },
});

const STREAMS = "tokuly.live.v1.LiveStreamService";
const CHAT = "tokuly.live.v1.LiveChatService";
const POLLS = "tokuly.live.v1.LivePollService";
const OVERLAYS = "tokuly.live.v1.OverlayService";
const CHANNELS = "tokuly.video.v1.ChannelService";
const WALLET = "tokuly.wallet.v1.WalletService";

// ───────────────────────────── Arranging state ─────────────────────────────

interface Stream {
  id: string;
  shortId: string;
  status: string;
}

interface StreamPage {
  summary: Stream;
  archiveStatus?: string;
  archiveVideoId?: string;
}

const suffix = () =>
  `${Date.now().toString(36).slice(-5)}${Math.random().toString(36).slice(2, 5)}`;

async function createChannel(request: APIRequestContext, owner: Account): Promise<string> {
  const response = await rpc<{ channel: { summary: { id: string } } }>(
    request,
    `${CHANNELS}/CreateChannel`,
    { handle: `live_${suffix()}`, name: `${owner.displayName} ch` },
    owner,
  );
  return response.channel.summary.id;
}

async function createStream(
  request: APIRequestContext,
  owner: Account,
  fields: Record<string, unknown> = {},
): Promise<Stream> {
  const response = await rpc<{ stream: StreamPage }>(
    request,
    `${STREAMS}/CreateStream`,
    { title: `配信テスト ${suffix()}`, ...fields },
    owner,
  );
  return response.stream.summary;
}

async function getStream(
  request: APIRequestContext,
  streamId: string,
  as?: Account,
): Promise<StreamPage> {
  return (await rpc<{ stream: StreamPage }>(request, `${STREAMS}/GetStream`, { streamId }, as))
    .stream;
}

/** Buys the 100-coin package with the development payment provider. */
async function grantCoins(request: APIRequestContext, to: Account): Promise<void> {
  const shop = await rpc<{ packages: { id: string; sku: string }[] }>(
    request,
    `${WALLET}/ListCoinPackages`,
    {},
    to,
  );
  const hundred = shop.packages.find((candidate) => candidate.sku === "coins_100");
  baseExpect(hundred, "the seeded coins_100 package").toBeTruthy();
  await rpc(request, `${WALLET}/StartPurchase`, { packageId: hundred?.id }, to);
}

async function coinsOf(request: APIRequestContext, account: Account): Promise<string> {
  return (
    (await rpc<{ balance: { coins?: string } }>(request, `${WALLET}/GetBalance`, {}, account))
      .balance.coins ?? "0"
  );
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

/**
 * Goes live from the control room with Chrome's fake camera: the page sends
 * the test pattern over WebRTC (WHIP) and the stream goes on air by itself.
 */
async function goLiveFromBrowser(
  page: Page,
  request: APIRequestContext,
  owner: Account,
  stream: Stream,
): Promise<void> {
  await page.goto(`/studio/live/${stream.shortId}`);
  const publisher = page.getByTestId("browser-publisher");
  await expect(publisher.getByTestId("publish-preview")).toBeVisible();
  const send = publisher.getByRole("button", { name: "映像を送る" });
  await expect(send).toBeEnabled();
  await send.click();
  await expect(publisher).toHaveAttribute("data-phase", "publishing", { timeout: 30_000 });
  await expect(async () => {
    baseExpect((await getStream(request, stream.id, owner)).summary.status).toBe(
      "STREAM_STATUS_LIVE",
    );
  }).toPass({ timeout: 45_000 });
  await expect(page.getByTestId("stream-status")).toHaveAttribute("data-status", "live");
}

/** Ends the stream from its control room, after the confirmation. */
async function endStream(page: Page): Promise<void> {
  await page.getByRole("button", { name: "配信を終了する" }).first().click();
  await page.getByRole("dialog").getByRole("button", { name: "配信を終了する" }).click();
  await expect(page.getByTestId("stream-status")).toHaveAttribute("data-status", "ended");
}

const composer = (page: Page) => page.getByRole("textbox", { name: "チャットにメッセージを書く" });

async function say(page: Page, text: string): Promise<void> {
  await composer(page).fill(text);
  await page.getByRole("button", { name: "送信" }).click();
}

const chatLines = (page: Page) => page.getByTestId("chat-list").getByTestId("chat-line");

// ───────────────────────────── Scheduling ─────────────────────────────

test("a stream scheduled in the studio shows under scheduled with a countdown for a friend", async ({
  page,
  request,
  me,
  browser,
}, testInfo) => {
  await createChannel(request, me);
  const friend = await createAccount(request, "livefan");
  await befriend(request, me, friend);
  const title = `予約配信 ${suffix()}`;

  await page.goto("/studio/live/new");
  await page.getByLabel("タイトル", { exact: true }).fill(title);
  await page.getByText("日時を決めて予約する").click();
  await expect(page.getByLabel("日付")).not.toHaveValue("");
  await page.getByRole("button", { name: "配信を予約する" }).click();
  await expect(page).toHaveURL(/\/studio\/live\/[A-Za-z0-9_-]{11}$/);
  await expect(page.getByTestId("stream-status")).toHaveAttribute("data-status", "upcoming");

  const viewer = await browserOf(browser, testInfo, friend);
  await viewer.goto("/live?tab=upcoming");
  const card = viewer.getByTestId("friend-upcoming").getByTestId("stream-card").filter({
    hasText: title,
  });
  await expect(card).toBeVisible();
  await expect(card.getByTestId("countdown")).toHaveText(/^あと /);

  await card.click();
  await expect(viewer.getByTestId("upcoming-stage").getByTestId("countdown")).toHaveText(/^あと /);
  await viewer.getByRole("button", { name: "通知を受け取る" }).click();
  await expect(viewer.getByRole("button", { name: "通知オン" })).toBeVisible();
  await viewer.context().close();
});

// ───────────────────────────── Going live ─────────────────────────────

test("going live from the browser: a viewer watches and is counted, and sees the end", async ({
  page,
  request,
  me,
  browser,
}, testInfo) => {
  await createChannel(request, me);
  const stream = await createStream(request, me);
  const watcher = await createAccount(request, "watcher");

  await goLiveFromBrowser(page, request, me, stream);

  const viewer = await browserOf(browser, testInfo, watcher);
  await viewer.goto(`/live/${stream.shortId}`);
  await expect(viewer.getByTestId("live-room")).toHaveAttribute("data-status", "2");
  await expect(viewer.getByTestId("live-player")).toBeVisible();
  await expect(async () => {
    const counts = await viewer
      .getByTestId("viewer-count")
      .evaluateAll((nodes) => nodes.map((node) => Number(node.getAttribute("data-count"))));
    baseExpect(Math.max(0, ...counts)).toBeGreaterThanOrEqual(1);
  }).toPass({ timeout: 45_000 });

  await endStream(page);
  await expect(viewer.getByTestId("live-room")).toHaveAttribute("data-status", "3");
  await expect(viewer.getByText("この配信は終了しました")).toBeVisible();
  await viewer.context().close();
});

test("the archive link appears once the recording of an ended stream is a video", async ({
  page,
  request,
  me,
}) => {
  // The archive job joins the recorded segments with ffmpeg's concat demuxer and an `inpoint` on
  // the first one; ffmpeg 8 refuses that for segments with Opus audio ("Operation not permitted"),
  // and Opus is what every browser publishes over WebRTC. RTMP (AAC) streams archive fine, but a
  // browser test can only publish over WebRTC. Backend request filed; enable this test with the fix.
  test.fixme(true, "live.archive fails for WebRTC (Opus) recordings with ffmpeg 8");
  await createChannel(request, me);
  const stream = await createStream(request, me);
  await goLiveFromBrowser(page, request, me, stream);
  await page.waitForTimeout(5000);
  await endStream(page);
  await page.goto(`/live/${stream.shortId}`);
  await expect(page.getByTestId("archive-link").first()).toBeVisible({ timeout: 120_000 });
});

// ───────────────────────────── Who may watch ─────────────────────────────

async function reasonOf(work: Promise<unknown>): Promise<string> {
  return work.then(
    () => "",
    (error: unknown) => (error instanceof RpcError ? error.reason : String(error)),
  );
}

test("a friends-only stream: a stranger cannot play it, and a friend who is unfriended leaves the player", async ({
  page,
  request,
  me,
  browser,
}, testInfo) => {
  await createChannel(request, me);
  const stream = await createStream(request, me, { visibility: "VISIBILITY_FRIENDS" });
  const friend = await createAccount(request, "closefan");
  const stranger = await createAccount(request, "stranger");
  await befriend(request, me, friend);
  await goLiveFromBrowser(page, request, me, stream);

  // A stranger gets what any missing stream gets: no page, no playback URLs.
  baseExpect(await reasonOf(getStream(request, stream.id, stranger))).toBe("STREAM_NOT_FOUND");
  const outsider = await browserOf(browser, testInfo, stranger);
  await outsider.goto(`/live/${stream.shortId}`);
  await expect(outsider.getByText("この配信は見つかりません")).toBeVisible();
  await expect(outsider.getByTestId("live-player")).toHaveCount(0);
  await outsider.context().close();

  const viewer = await browserOf(browser, testInfo, friend);
  await viewer.goto(`/live/${stream.shortId}`);
  await expect(viewer.getByTestId("live-player")).toHaveAttribute("data-playing", "true", {
    timeout: 45_000,
  });

  // The friendship ends: the media server cuts the session and the page lands where a stranger's does.
  await rpc(request, "tokuly.social.v1.FriendService/RemoveFriend", { userId: friend.userId }, me);
  await expect(viewer.getByText("この配信は見つかりません")).toBeVisible({ timeout: 40_000 });
  await expect(viewer.getByTestId("live-player")).toHaveCount(0);
  await viewer.context().close();
});

test("only the owner ever receives the stream key and ingest addresses", async ({
  page,
  request,
  me,
}) => {
  const owner = await createAccount(request, "keyowner");
  await createChannel(request, owner);
  const stream = await createStream(request, owner);
  const { ingest } = await rpc<{ ingest: { streamKey: string } }>(
    request,
    `${STREAMS}/GetIngestInfo`,
    { streamId: stream.id },
    owner,
  );
  baseExpect(ingest.streamKey.length).toBeGreaterThan(8);
  baseExpect(
    await reasonOf(rpc(request, `${STREAMS}/GetIngestInfo`, { streamId: stream.id }, me)),
  ).toBe("STREAM_NOT_FOUND");

  // Everything the browser receives on the watch page and the control room, as someone else.
  const bodies: Promise<string>[] = [];
  page.on("response", (response) => {
    if (response.url().includes("/api/")) bodies.push(response.text().catch(() => ""));
  });
  await page.goto(`/live/${stream.shortId}`);
  await expect(page.getByTestId("live-room")).toBeVisible();
  await page.goto(`/studio/live/${stream.shortId}`);
  await expect(page.getByText("この配信のコントロールルームは開けません")).toBeVisible();
  await expect(page.getByTestId("secret-field")).toHaveCount(0);
  await expect(page.getByTestId("ingest-panel")).toHaveCount(0);
  const html = await page.content();
  baseExpect(html).not.toContain(ingest.streamKey);
  for (const body of await Promise.all(bodies)) baseExpect(body).not.toContain(ingest.streamKey);
});

// ───────────────────────────── Chat ─────────────────────────────

test("chat lines arrive live, the streamer deletes one, and slow mode holds a second message back", async ({
  page,
  request,
  me,
  browser,
}, testInfo) => {
  await createChannel(request, me);
  const stream = await createStream(request, me);
  const fan = await createAccount(request, "chatfan");

  const viewer = await browserOf(browser, testInfo, fan);
  await viewer.goto(`/live/${stream.shortId}`);
  await page.goto(`/live/${stream.shortId}`);
  await expect(composer(page)).toBeVisible();
  await expect(composer(viewer)).toBeVisible();

  await say(viewer, "こんばんは〜！");
  await expect(chatLines(page).filter({ hasText: "こんばんは〜！" })).toBeVisible();
  await say(page, "いらっしゃい！");
  await expect(chatLines(viewer).filter({ hasText: "いらっしゃい！" })).toBeVisible();
  await say(viewer, "これは消される");
  const doomed = chatLines(page).filter({ hasText: "これは消される" });
  await expect(doomed).toBeVisible();

  // The streamer moderates it away; it leaves everyone's chat.
  await doomed.getByRole("button", { name: "メッセージの操作" }).click();
  await page.getByRole("dialog").getByRole("button", { name: "メッセージを削除" }).click();
  await expect(chatLines(viewer).filter({ hasText: "これは消される" })).toHaveCount(0);
  await expect(chatLines(page).filter({ hasText: "これは消される" })).toHaveCount(0);

  // Slow mode: one message, then the composer waits.
  await rpc(
    request,
    `${CHAT}/UpdateChatSettings`,
    {
      streamId: stream.id,
      chat: {
        enabled: true,
        slowModeSeconds: 60,
        audience: "CHAT_AUDIENCE_EVERYONE",
        linkPolicy: "LINK_POLICY_ALLOW",
        stampsEnabled: true,
      },
    },
    me,
  );
  await expect(viewer.getByText("低速 60秒")).toBeVisible();
  await say(viewer, "ひとつめ");
  await expect(chatLines(page).filter({ hasText: "ひとつめ" })).toBeVisible();
  await composer(viewer).fill("ふたつめ");
  await expect(viewer.getByTestId("slow-mode-wait")).toBeVisible();
  await expect(viewer.getByRole("button", { name: "送信" })).toBeDisabled();
  await expect(chatLines(page).filter({ hasText: "ふたつめ" })).toHaveCount(0);
  // Not only the button: the server refuses a second line within the window, with its own reason.
  const refused = await rpc(
    request,
    `${CHAT}/SendChatMessage`,
    { streamId: stream.id, text: "すりぬけ" },
    fan,
  ).then(
    () => "",
    (error: unknown) => (error instanceof RpcError ? error.reason : String(error)),
  );
  baseExpect(refused).toBe("SLOW_MODE");
  await viewer.context().close();
});

/**
 * Stands between the page and the realtime gateway and holds back the first
 * frame that asks for `topic` (the Hello naming it, or its Subscribe), with
 * everything the page sends after it, until `release()`. Until then the
 * gateway does not have the subscription: what is published on the topic in
 * the meantime never reaches the page, and only a read made after the
 * gateway's answer has it. The page is kept on WebSocket, which Playwright
 * can stand in.
 */
async function holdSubscription(page: Page, topic: string) {
  const needle = Buffer.from(topic);
  let holding = true;
  const held: (() => void)[] = [];
  await page.addInitScript(() => {
    Reflect.deleteProperty(globalThis, "WebTransport");
  });
  await page.routeWebSocket(/\/rt\/ws/, (ws) => {
    const server = ws.connectToServer();
    ws.onMessage((message) => {
      const bytes = typeof message === "string" ? Buffer.from(message) : message;
      if (holding && (held.length > 0 || bytes.includes(needle)))
        held.push(() => server.send(message));
      else server.send(message);
    });
  });
  return {
    asked: () => held.length > 0,
    release() {
      holding = false;
      for (const send of held.splice(0)) send();
    },
  };
}

test("a chat line sent while the room is still subscribing shows once the subscription is in place", async ({
  page,
  request,
  me,
}) => {
  await createChannel(request, me);
  const stream = await createStream(request, me);
  await rpc(request, `${CHAT}/SendChatMessage`, { streamId: stream.id, text: "さきに来てた" }, me);
  const gate = await holdSubscription(page, `live:${stream.id}`);

  await page.goto(`/live/${stream.shortId}`);
  // The chat was read; the page asked for the topic, which the gateway has not heard yet.
  await expect(chatLines(page).filter({ hasText: "さきに来てた" })).toBeVisible();
  await expect.poll(gate.asked).toBe(true);
  await rpc(
    request,
    `${CHAT}/SendChatMessage`,
    { streamId: stream.id, text: "すきまに来た！" },
    me,
  );

  // The gateway answers once the subscription is in place; the room reads again then.
  gate.release();
  await expect(chatLines(page).filter({ hasText: "すきまに来た！" })).toBeVisible();
  // And from now on lines arrive live.
  await rpc(request, `${CHAT}/SendChatMessage`, { streamId: stream.id, text: "そのあと！" }, me);
  await expect(chatLines(page).filter({ hasText: "そのあと！" })).toBeVisible();
});

test("a pin moved or deleted while the stream is read again stays as pushed", async ({
  page,
  request,
  me,
}) => {
  await createChannel(request, me);
  const stream = await createStream(request, me);
  const send = async (text: string) =>
    (
      await rpc<{ message: { id: string } }>(
        request,
        `${CHAT}/SendChatMessage`,
        { streamId: stream.id, text },
        me,
      )
    ).message.id;
  const pin = (messageId: string) => rpc(request, `${CHAT}/PinChatMessage`, { messageId }, me);
  const first = await send("さいしょのピン");
  const second = await send("つぎのピン");
  await pin(first);

  await page.goto(`/live/${stream.shortId}`);
  const pinned = page.getByTestId("pinned-message");
  await expect(pinned).toContainText("さいしょのピン");
  // Lines arrive live: the room is subscribed.
  await send("つながった！");
  await expect(chatLines(page).filter({ hasText: "つながった！" })).toBeVisible();

  // New chat rules make the page read the stream again; that answer is slow.
  const rules = (slowModeSeconds: number) =>
    rpc(
      request,
      `${CHAT}/UpdateChatSettings`,
      {
        streamId: stream.id,
        chat: {
          enabled: true,
          slowModeSeconds,
          audience: "CHAT_AUDIENCE_EVERYONE",
          linkPolicy: "LINK_POLICY_ALLOW",
          stampsEnabled: true,
        },
      },
      me,
    );
  let reads = await holdAnswers(page, `${STREAMS}/GetStream`);
  await rules(30);
  await expect.poll(reads.held).toBeGreaterThan(0);
  // Pushed after that answer was made: the pin moves.
  await pin(second);
  await expect(pinned).toContainText("つぎのピン");
  await reads.release();
  await expect(pinned).toContainText("つぎのピン");

  // Again, and this time the moderator deletes the pinned line.
  reads = await holdAnswers(page, `${STREAMS}/GetStream`);
  await rules(0);
  await expect.poll(reads.held).toBeGreaterThan(0);
  await rpc(request, `${CHAT}/DeleteChatMessage`, { messageId: second }, me);
  await expect(pinned).toHaveCount(0);
  await reads.release();
  await expect(pinned).toHaveCount(0);
  await expect(chatLines(page).filter({ hasText: "つぎのピン" })).toHaveCount(0);
});

// ───────────────────────────── Polls ─────────────────────────────

test("a poll round trip: the streamer opens it, a viewer votes, the results move for both", async ({
  page,
  request,
  me,
  browser,
}, testInfo) => {
  await createChannel(request, me);
  const stream = await createStream(request, me);
  const fan = await createAccount(request, "pollfan");

  const viewer = await browserOf(browser, testInfo, fan);
  await viewer.goto(`/live/${stream.shortId}`);
  await page.goto(`/live/${stream.shortId}`);

  await page.getByRole("button", { name: "アンケートをはじめる" }).click();
  const dialog = page.getByRole("dialog");
  await dialog.getByLabel("質問").fill("今日のおやつは？");
  await dialog.getByLabel("選択肢 1").fill("たいやき");
  await dialog.getByLabel("選択肢 2").fill("クレープ");
  await dialog.getByRole("button", { name: "スタート！" }).click();

  const card = viewer.getByTestId("poll-card");
  await expect(card).toContainText("今日のおやつは？");
  await card.getByRole("button", { name: "クレープ" }).click();
  await expect(card.getByTestId("poll-share").last()).toHaveText("100%");
  await expect(page.getByTestId("poll-card").getByTestId("poll-votes")).toHaveText("1票");

  const polls = await rpc<{ polls: { totalVotes?: string }[] }>(
    request,
    `${POLLS}/ListPolls`,
    { streamId: stream.id },
    me,
  );
  baseExpect(polls.polls[0]?.totalVotes).toBe("1");
  await viewer.context().close();
});

// ───────────────────────────── Gifts ─────────────────────────────

test("a gift lowers the sender's coins and shows highlighted in the chat", async ({
  page,
  request,
  me,
  browser,
}, testInfo) => {
  const streamer = await createAccount(request, "giftee");
  await createChannel(request, streamer);
  const stream = await createStream(request, streamer);
  await grantCoins(request, me);

  const studio = await browserOf(browser, testInfo, streamer);
  await goLiveFromBrowser(studio, request, streamer, stream);

  await page.goto(`/live/${stream.shortId}`);
  await page.getByRole("button", { name: "ギフトを贈る" }).click();
  const dialog = page.getByRole("dialog");
  await expect(dialog.getByTestId("gift-balance")).toHaveAttribute("data-coins", "100");
  await dialog.getByLabel("コインの数").fill("50");
  await dialog.getByLabel("メッセージ").fill("いつも応援してるよ！");
  await expect(dialog.getByTestId("gift-message")).toContainText("いつも応援してるよ！");
  await dialog.getByRole("button", { name: "確認へすすむ" }).click();
  await dialog.getByRole("button", { name: "50コインを贈る" }).click();
  await expect(dialog).toBeHidden();

  const line = page.getByTestId("chat-list").getByTestId("gift-message");
  await expect(line).toContainText("いつも応援してるよ！");
  await expect(line).toHaveAttribute("data-tier", "2");
  // On phones the control room shows its chat behind a tab.
  await studio
    .getByRole("tab", { name: "チャット", exact: true })
    .locator("visible=true")
    .first()
    .click();
  await expect(studio.getByTestId("chat-list").getByTestId("gift-message")).toContainText(
    "いつも応援してるよ！",
  );
  expect(await coinsOf(request, me)).toBe("50");
  await studio.context().close();
});

test("a streamer under 18 is told that gifts come from 18 on", async ({ page, request }) => {
  const streamer = await createAccount(request, "teen", { birthdate: "2010-05-01" });
  await createChannel(request, streamer);
  const stream = await createStream(request, streamer);
  await signInAs(page, streamer);

  await page.goto(`/live/${stream.shortId}`);
  await expect(page.getByTestId("gifts-age-restricted")).toContainText(
    "投げ銭・ギフトは18歳から受け取れるよ！",
  );
  await page.goto("/studio/live/new");
  await expect(page.getByText("投げ銭・ギフトは18歳から受け取れるよ！")).toBeVisible();
});

// ───────────────────────────── Overlays ─────────────────────────────

test("the chat overlay shows a message on a bare page that keeps its address to itself", async ({
  page,
  request,
  me,
  browser,
}, testInfo) => {
  const channelId = await createChannel(request, me);
  const stream = await createStream(request, me);
  // A chat box follows the channel's broadcast that is on air.
  const studio = await browserOf(browser, testInfo, me);
  await goLiveFromBrowser(studio, request, me, stream);
  const { overlay } = await rpc<{ overlay: { token: string } }>(
    request,
    `${OVERLAYS}/CreateOverlay`,
    { channelId, kind: "OVERLAY_KIND_CHAT_BOX", name: "テスト用チャット" },
    me,
  );
  await rpc(
    request,
    `${CHAT}/SendChatMessage`,
    { streamId: stream.id, text: "オーバーレイに出るかな" },
    me,
  );

  const response = await page.goto(`/overlay/${overlay.token}`);
  baseExpect(response?.headers()["referrer-policy"]).toBe("no-referrer");
  // OBS and streaming tools may frame the widget; every other page refuses that.
  baseExpect(response?.headers()["x-frame-options"]).toBeUndefined();
  baseExpect(response?.headers()["content-security-policy"]).toContain("frame-ancestors *");
  const box = page.getByTestId("overlay-chat");
  await expect(box).toContainText("オーバーレイに出るかな");

  // New lines arrive live.
  await rpc(
    request,
    `${CHAT}/SendChatMessage`,
    { streamId: stream.id, text: "ライブで追加！" },
    me,
  );
  await expect(box).toContainText("ライブで追加！");

  // Nothing of the app around it.
  await expect(page.getByRole("navigation")).toHaveCount(0);
  await expect(page.getByText("マップ")).toHaveCount(0);
  const background = await page.evaluate(() => getComputedStyle(document.body).backgroundColor);
  baseExpect(background).toMatch(/rgba\(0, 0, 0, 0\)|transparent/);
  await studio.context().close();
});
