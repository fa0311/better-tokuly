import { fileURLToPath } from "node:url";
import zlib from "node:zlib";
import type { APIRequestContext, Browser, Page, WebSocketRoute } from "@playwright/test";
import { type Account, befriend, createAccount, expect, rpc, signInAs, test } from "./support";

// These tests upload pictures, wait for the server to process them, and use two browsers.
test.describe.configure({ timeout: 120_000 });

const STORIES = "tokuly.notes.v1.StoryService";
const MEDIA = "tokuly.media.v1.MediaService";

const tag = () => `${Date.now().toString(36).slice(-5)}${Math.random().toString(36).slice(2, 6)}`;

/**
 * The dev server reports its own hiccups (a proxied connection reset when
 * some other browser closed) as an overlay on every open page, and the
 * overlay swallows the next click. It says nothing about the app: remove it
 * whenever it is in the way.
 */
async function dismissDevServerOverlay(page: Page): Promise<void> {
  await page.addLocatorHandler(page.locator("vite-error-overlay"), async (overlay) => {
    await overlay.evaluate((element) => element.remove());
  });
}

test.beforeEach(async ({ page }) => dismissDevServerOverlay(page));

/** A plain PNG picture. */
function picture(
  name: string,
  [red, green, blue]: [number, number, number],
  width = 360,
  height = 640,
) {
  const chunk = (type: string, data: Buffer) => {
    const body = Buffer.concat([Buffer.from(type, "latin1"), data]);
    const out = Buffer.alloc(body.length + 8);
    out.writeUInt32BE(data.length, 0);
    body.copy(out, 4);
    out.writeUInt32BE(zlib.crc32(body) >>> 0, body.length + 4);
    return out;
  };
  const header = Buffer.alloc(13);
  header.writeUInt32BE(width, 0);
  header.writeUInt32BE(height, 4);
  header.set([8, 2, 0, 0, 0], 8); // 8-bit RGB
  const row = Buffer.concat([
    Buffer.from([0]),
    Buffer.alloc(width * 3, Buffer.from([red, green, blue])),
  ]);
  const pixels = Buffer.concat(Array.from({ length: height }, () => row));
  const signature = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
  return {
    name,
    mimeType: "image/png",
    buffer: Buffer.concat([
      signature,
      chunk("IHDR", header),
      chunk("IDAT", zlib.deflateSync(pixels)),
      chunk("IEND", Buffer.alloc(0)),
    ]),
  };
}

/** Uploads a picture for stories through the API and waits until it can be attached. */
async function uploadPicture(
  request: APIRequestContext,
  account: Account,
  color: [number, number, number],
): Promise<string> {
  const file = picture("story.png", color);
  const created = await rpc<{
    uploadId: string;
    putUrl: string;
    putHeaders?: Record<string, string>;
  }>(
    request,
    `${MEDIA}/CreateUpload`,
    {
      purpose: "ASSET_PURPOSE_STORY",
      fileName: file.name,
      contentType: file.mimeType,
      sizeBytes: String(file.buffer.length),
    },
    account,
  );
  const put = await request.put(created.putUrl, {
    data: file.buffer,
    headers: created.putHeaders ?? {},
  });
  expect(put.ok(), "upload to object storage").toBeTruthy();
  const completed = await rpc<{ asset: { id: string } }>(
    request,
    `${MEDIA}/CompleteUpload`,
    { uploadId: created.uploadId },
    account,
  );
  await expect(async () => {
    const asset = (
      await rpc<{ asset: { status: string } }>(
        request,
        `${MEDIA}/GetAsset`,
        { assetId: completed.asset.id },
        account,
      )
    ).asset;
    expect(asset.status).toBe("ASSET_STATUS_READY");
  }).toPass({ timeout: 60_000 });
  return completed.asset.id;
}

/** Posts a story through the API: one picture per caption. */
async function postStory(
  request: APIRequestContext,
  account: Account,
  title: string,
  captions = ["e2e ひとこと"],
): Promise<{ id: string }> {
  const items = [];
  for (const caption of captions)
    items.push({ assetId: await uploadPicture(request, account, [255, 92, 157]), caption });
  return (
    await rpc<{ story: { id: string } }>(
      request,
      `${STORIES}/CreateStory`,
      { title, items },
      account,
    )
  ).story;
}

/** A second browser, signed in as `account`, the same size as the test's own page. */
async function browserFor(
  browser: Browser,
  page: Page,
  baseURL: string | undefined,
  account: Account,
): Promise<Page> {
  const context = await browser.newContext({
    baseURL,
    locale: "ja-JP",
    timezoneId: "Asia/Tokyo",
    viewport: page.viewportSize(),
    hasTouch: (page.viewportSize()?.width ?? 1280) < 700,
  });
  const other = await context.newPage();
  await dismissDevServerOverlay(other);
  await signInAs(other, account);
  return other;
}

test("a story is posted, a friend watches it live, and the owner sees who watched", async ({
  page,
  me,
  request,
  browser,
  baseURL,
}) => {
  const friend = await createAccount(request, "friend");
  await befriend(request, me, friend);

  // The friend has the timeline open before anything is posted.
  const theirs = await browserFor(browser, page, baseURL, friend);
  await theirs.goto("/notes?tab=home");
  await expect(theirs.getByTestId("story-tray")).toContainText("ともだちの記憶がここにならぶよ!");

  // Post a story: one picture, a caption, a title.
  await page.goto("/notes?tab=home");
  await page.getByTestId("story-add").click();
  const dialog = page.getByRole("dialog");
  await page.getByTestId("story-files").setInputFiles(picture("umi.png", [79, 195, 255]));
  await expect(page.locator('[data-testid="upload"][data-status="ready"]')).toBeVisible({
    timeout: 60_000,
  });
  const title = `e2e うみにきた ${tag()}`;
  await dialog.getByPlaceholder("ひとこと (なくてもOK)").fill("あおい!");
  await dialog.getByPlaceholder("今日はどんな日?").fill(title);
  await dialog.getByRole("button", { name: "のこす", exact: true }).click();
  await expect(page.getByText("記憶をのこしたよ!")).toBeVisible();
  await expect(page.getByTestId("story-reel-own")).toContainText("あなたの記憶");

  // It shows up in the friend's tray by itself, unseen.
  const reel = theirs.getByTestId("story-reel");
  await expect(reel).toHaveAttribute("data-unseen", "true", { timeout: 30_000 });
  await expect(reel).toContainText(me.displayName);

  // Watching it: the title and the caption are there; afterwards the ring is quiet.
  await reel.click();
  const viewer = theirs.getByTestId("story-viewer");
  await expect(viewer.getByText(title)).toBeVisible();
  await expect(viewer.getByText("あおい!")).toBeVisible();
  await expect(reel).toHaveAttribute("data-unseen", "false", { timeout: 20_000 });
  await viewer.getByRole("button", { name: "🔥" }).click();
  await expect(viewer.getByRole("button", { name: "🔥" })).toHaveAttribute("aria-pressed", "true");
  await viewer.getByRole("button", { name: "閉じる" }).click();
  await expect(viewer).toHaveCount(0);

  // The owner sees the view arrive, and who it was.
  await page.getByTestId("story-reel-own").click();
  const views = page.getByTestId("story-views");
  await expect(views).toContainText("1人が見たよ", { timeout: 30_000 });
  await views.click();
  const viewers = page.getByTestId("story-viewers");
  await expect(viewers).toContainText(friend.displayName);
  await expect(viewers.getByRole("img", { name: /🔥/ })).toBeVisible();
  // The list is on top of the story: a row can be followed to the profile.
  await viewers.getByRole("link", { name: new RegExp(friend.handle) }).click();
  await expect(page).toHaveURL(new RegExp(`/u/${friend.handle}$`));
  await expect(page.getByTestId("story-viewer")).toHaveCount(0);

  await theirs.context().close();
});

test("a story plays from its own address and ends when it was the last", async ({
  page,
  me,
  request,
}) => {
  const friend = await createAccount(request, "friend");
  await befriend(request, me, friend);
  const title = `e2e リンクからの記憶 ${tag()}`;
  const story = await postStory(request, friend, title);

  await page.goto(`/stories/${story.id}`);
  const viewer = page.getByTestId("story-viewer");
  await expect(viewer.getByText(title)).toBeVisible();
  await expect(viewer.getByText("e2e ひとこと")).toBeVisible();

  // A tap on the right goes on; after the last piece the viewer closes.
  const stage = page.getByTestId("story-stage");
  const box = (await stage.boundingBox())!;
  await page.mouse.click(box.x + box.width * 0.8, box.y + box.height * 0.5);
  await expect(viewer).toHaveCount(0);

  // A story that is gone says so.
  await rpc(request, `${STORIES}/DeleteStory`, { storyId: story.id }, friend);
  await page.goto(`/stories/${story.id}`);
  // A full page load and a lookup: slow on a busy machine.
  await expect(page.getByText("この記憶はもう見られないよ")).toBeVisible({ timeout: 30_000 });
});

test("a highlight keeps a story on the profile, for friends too", async ({
  page,
  me,
  request,
  browser,
  baseURL,
}) => {
  const friend = await createAccount(request, "friend");
  await befriend(request, me, friend);
  const title = `e2e ハイライトにする記憶 ${tag()}`;
  await postStory(request, me, title);

  await page.goto(`/u/${me.handle}`);
  const highlights = page.getByTestId("highlights");
  await highlights.getByRole("button", { name: "新規" }).click();
  const dialog = page.getByRole("dialog");
  const name = `なつ${tag().slice(0, 4)}`;
  await dialog.getByLabel("なまえ").fill(name);
  // Nothing picked yet: the highlight cannot be created.
  await expect(dialog.getByRole("button", { name: "つくる" })).toBeDisabled();
  await page.getByTestId("highlight-stories").getByRole("checkbox").first().click();
  await dialog.getByRole("button", { name: "つくる" }).click();
  await expect(page.getByText("ハイライトをつくったよ!")).toBeVisible();
  await expect(highlights.getByTestId("highlight")).toContainText(name);

  // The owner's face carries the story ring while the story is live.
  await expect(page.getByTestId("story-ring")).toBeVisible();

  // A friend finds the highlight on the profile and plays it.
  const theirs = await browserFor(browser, page, baseURL, friend);
  await theirs.goto(`/u/${me.handle}`);
  await theirs.getByTestId("highlights").getByTestId("highlight").click();
  await expect(theirs.getByTestId("story-viewer").getByText(title)).toBeVisible();
  await theirs.context().close();

  // Rename it from the manage list, then delete it.
  await highlights.getByRole("button", { name: "編集", exact: true }).click();
  await page.getByRole("button", { name: `「${name}」を編集` }).click();
  await page.getByRole("dialog").getByLabel("なまえ").fill(`${name}2`);
  await page.getByRole("dialog").getByRole("button", { name: "保存する" }).click();
  await expect(page.getByText("ハイライトを保存したよ!")).toBeVisible();
  await expect(highlights.getByTestId("highlight")).toContainText(`${name}2`);

  // Saving returns to the manage list.
  await page.getByRole("button", { name: `「${name}2」を削除` }).click();
  await page.getByRole("dialog").last().getByRole("button", { name: "削除する" }).click();
  await expect(page.getByText("ハイライトを削除したよ")).toBeVisible();
  await expect(highlights.getByTestId("highlight")).toHaveCount(0);
});

test("the archive keeps stories by day, and a story is deleted from the viewer", async ({
  page,
  me,
  request,
}) => {
  const title = `e2e アーカイブの記憶 ${tag()}`;
  await postStory(request, me, title);

  await page.goto("/stories/archive");
  const tile = page.locator('[data-testid="archived-story"]', { hasText: title });
  await expect(tile).toBeVisible();
  await expect(tile).toContainText("公開中");

  // The calendar marks today; picking the day narrows the list to it.
  const day = page.getByTestId("story-day");
  await expect(day).toHaveCount(1);
  await day.click();
  await expect(day).toHaveAttribute("aria-pressed", "true");
  await expect(tile).toBeVisible();

  // Open it and delete it from the viewer's menu.
  await tile.click();
  const viewer = page.getByTestId("story-viewer");
  await expect(viewer.getByText(title)).toBeVisible();
  await viewer.getByRole("button", { name: "メニュー" }).click();
  await page.getByRole("menuitem", { name: "記憶を削除" }).click();
  await page.getByRole("dialog").last().getByRole("button", { name: "記憶を削除" }).click();
  await expect(page.getByText("記憶を削除したよ")).toBeVisible();
  await expect(viewer).toHaveCount(0);
  await expect(tile).toHaveCount(0);
  await expect(page.getByText("記憶はまだないよ")).toBeVisible();
});

test("the keyboard moves through a story, wherever the focus is in the viewer", async ({
  page,
  me,
  request,
}) => {
  const friend = await createAccount(request, "friend");
  await befriend(request, me, friend);
  const story = await postStory(request, friend, `e2e キーボードの記憶 ${tag()}`, [
    "いちまいめ",
    "にまいめ",
  ]);

  await page.goto(`/stories/${story.id}`);
  const viewer = page.getByTestId("story-viewer");
  await expect(viewer.getByText("いちまいめ")).toBeVisible({ timeout: 30_000 });

  // Space holds the story, so the keys below are not racing its clock.
  await page.keyboard.press("Space");
  await expect(viewer).toHaveAttribute("data-paused", "true");
  await page.keyboard.press("ArrowRight");
  await expect(viewer.getByText("にまいめ")).toBeVisible();
  await page.keyboard.press("ArrowLeft");
  await expect(viewer.getByText("いちまいめ")).toBeVisible();

  // With the focus on a control inside the viewer the arrows still work.
  await viewer.getByRole("button", { name: "🔥" }).focus();
  await page.keyboard.press("ArrowRight");
  await expect(viewer.getByText("にまいめ")).toBeVisible();

  // A menu opened from the viewer keeps its own arrow keys.
  await viewer.getByRole("button", { name: "メニュー" }).click();
  await expect(page.getByRole("menuitem", { name: "通報する" })).toBeVisible();
  await page.keyboard.press("ArrowRight");
  await page.keyboard.press("Escape");
  await expect(page.getByRole("menuitem", { name: "通報する" })).toHaveCount(0);
  await expect(viewer.getByText("にまいめ")).toBeVisible();

  // Past the last piece the viewer closes.
  await viewer.getByRole("button", { name: "🔥" }).focus();
  await page.keyboard.press("ArrowRight");
  await expect(viewer).toHaveCount(0);
});

test("blocking someone takes their notes and stories off what is already loaded", async ({
  page,
  me,
  request,
}) => {
  const friend = await createAccount(request, "friend");
  await befriend(request, me, friend);
  const rules = await rpc<{ version: number }>(
    request,
    "tokuly.notes.v1.NoteService/GetNoteRules",
    {},
    friend,
  );
  await rpc(
    request,
    "tokuly.notes.v1.NoteService/AgreeToNoteRules",
    { version: rules.version },
    friend,
  );
  const text = `e2e ブロックされる人のノート ${tag()}`;
  await rpc(
    request,
    "tokuly.notes.v1.NoteService/CreateNote",
    { text, visibility: "VISIBILITY_FRIENDS" },
    friend,
  );
  await postStory(request, friend, `e2e ブロックされる人の記憶 ${tag()}`);

  await page.goto("/notes?tab=home");
  const note = page.locator('[data-testid="note"]', { hasText: text });
  await expect(note).toBeVisible();
  await expect(page.getByTestId("story-reel")).toHaveCount(1);

  // To their profile and back without reloading: what was loaded stays loaded.
  await note.getByRole("link", { name: friend.displayName, exact: true }).first().click();
  await expect(page).toHaveURL(new RegExp(`/u/${friend.handle}$`));
  page.once("dialog", (dialog) => void dialog.accept());
  await page.getByRole("button", { name: "ブロック" }).click();
  await expect(page).toHaveURL(/\/map$/);

  await page
    .getByRole("navigation", { name: "main" })
    .getByRole("link", { name: "ノート" })
    .first()
    .click();
  await page.getByRole("tab", { name: "ともだち" }).click();
  await expect(page.getByTestId("timeline")).toBeVisible();
  await expect(note).toHaveCount(0);
  await expect(page.getByTestId("story-reel")).toHaveCount(0);
});

test("when a friend ends the friendship, their notes and their story leave the open timeline", async ({
  page,
  me,
  request,
}) => {
  const friend = await createAccount(request, "friend");
  await befriend(request, me, friend);
  const rules = await rpc<{ version: number }>(
    request,
    "tokuly.notes.v1.NoteService/GetNoteRules",
    {},
    friend,
  );
  await rpc(
    request,
    "tokuly.notes.v1.NoteService/AgreeToNoteRules",
    { version: rules.version },
    friend,
  );
  const text = `e2e もと友達のノート ${tag()}`;
  await rpc(
    request,
    "tokuly.notes.v1.NoteService/CreateNote",
    { text, visibility: "VISIBILITY_FRIENDS" },
    friend,
  );
  await postStory(request, friend, `e2e もと友達の記憶 ${tag()}`);

  await page.goto("/notes?tab=home");
  const note = page.locator('[data-testid="note"]', { hasText: text });
  await expect(note).toBeVisible();
  await page.getByTestId("story-reel").click();
  await expect(page.getByTestId("story-viewer")).toBeVisible();

  // The friend removes the reader: the realtime connection says so.
  await rpc(request, "tokuly.social.v1.FriendService/RemoveFriend", { userId: me.userId }, friend);
  await expect(page.getByTestId("story-viewer")).toHaveCount(0, { timeout: 30_000 });
  await expect(page.getByTestId("story-reel")).toHaveCount(0);
  await expect(note).toHaveCount(0);
});

/**
 * Keeps the page's realtime connection on WebSocket, which Playwright can stand in. `drop()`
 * closes it; what it returns says whether the connection after it was welcomed yet.
 */
async function droppableConnection(page: Page) {
  await page.addInitScript(() => {
    Reflect.deleteProperty(globalThis, "WebTransport");
  });
  const sockets: { ws: WebSocketRoute; welcomed: boolean }[] = [];
  await page.routeWebSocket(/\/rt\/ws/, (ws) => {
    const socket = { ws, welcomed: false };
    sockets.push(socket);
    const server = ws.connectToServer();
    ws.onMessage((message) => server.send(message));
    server.onMessage((message) => {
      socket.welcomed = true;
      ws.send(message);
    });
  });
  return {
    async drop() {
      await expect.poll(() => sockets.at(-1)?.welcomed).toBe(true);
      const next = sockets.length;
      await sockets.at(-1)?.ws.close();
      return () => sockets[next]?.welcomed === true;
    },
  };
}

test("a story deleted while the tray was read again does not come back with that read", async ({
  page,
  me,
  request,
}) => {
  const friend = await createAccount(request, "friend");
  await befriend(request, me, friend);
  const story = await postStory(request, friend, `e2e すぐ消える記憶 ${tag()}`);
  const connection = await droppableConnection(page);
  await page.goto("/notes?tab=home");
  await expect(page.getByTestId("story-reel")).toBeVisible();

  // The connection drops; the Welcome after it reads the tray again. That read is answered now,
  // while the story is there, and lands only after the story was deleted.
  const reconnected = await connection.drop();
  let answered = () => {};
  const read = new Promise<void>((resolve) => (answered = resolve));
  let release = () => {};
  const released = new Promise<void>((resolve) => (release = resolve));
  await page.route(`**/api/${STORIES}/GetStoryTray`, async (route) => {
    if (!reconnected()) return route.continue();
    const response = await route.fetch();
    answered();
    await released;
    await route.fulfill({ response });
  });
  await read;
  await rpc(request, `${STORIES}/DeleteStory`, { storyId: story.id }, friend);
  await expect(page.getByTestId("story-reel")).toHaveCount(0, { timeout: 30_000 });

  const landed = page.waitForEvent("requestfinished", (finished) =>
    finished.url().endsWith(`${STORIES}/GetStoryTray`),
  );
  release();
  await landed;
  await page.waitForTimeout(500);
  await expect(page.getByTestId("story-reel")).toHaveCount(0);
});

/**
 * A 62-second MP4 of 64x64 at one frame per second (about 3 KB), checked in so the test needs
 * no ffmpeg on the machine. Made with:
 * `ffmpeg -f lavfi -i color=c=0x58cc02:size=64x64:rate=1:duration=62 -c:v libx264 -preset veryslow
 *  -pix_fmt yuv420p -g 1000 -movflags +faststart -map_metadata -1 -fflags +bitexact story-62s.mp4`
 */
const longVideo = fileURLToPath(new URL("./fixtures/story-62s.mp4", import.meta.url));

test("a story video longer than a minute is refused, saying how long it may be", async ({
  page,
  me: _me,
}) => {
  await page.goto("/notes?tab=home");
  await page.getByTestId("story-add").click();
  await page.getByTestId("story-files").setInputFiles(longVideo);
  await expect(page.getByText("長すぎるよ!60秒までにしてもう一度ためしてね")).toBeVisible({
    timeout: 60_000,
  });
  await expect(page.locator('[data-testid="upload"]')).toHaveCount(0);
});
