import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import type { APIRequestContext, Browser, Locator, Page } from "@playwright/test";
import {
  type Account,
  test as baseTest,
  befriend,
  createAccount,
  expect,
  rpc,
  signInAs,
} from "./support";

const CHANNELS = "tokuly.video.v1.ChannelService";
const SHORTS = "tokuly.shorts.v1.ShortService";
const MEDIA = "tokuly.media.v1.MediaService";

// ───────────────────────────── Arranging state ─────────────────────────────

/** Length of the test video in seconds: long enough to count as watched, short enough to loop within a test. */
const VIDEO_SECONDS = 4;

/** Records a moving vertical test picture with a tone in the browser: the way to a test video on machines without ffmpeg. */
async function recordVideo(browser: Browser, seconds: number): Promise<Buffer> {
  const page = await browser.newPage();
  try {
    const base64 = await page.evaluate(async (length) => {
      const canvas = document.createElement("canvas");
      canvas.width = 180;
      canvas.height = 320;
      const context = canvas.getContext("2d") as CanvasRenderingContext2D;
      const started = performance.now();
      const paint = () => {
        const elapsed = (performance.now() - started) / 1000;
        context.fillStyle = `hsl(${(elapsed * 60) % 360} 80% 50%)`;
        context.fillRect(0, 0, 180, 320);
        context.fillStyle = "#fff";
        context.fillRect(60, ((elapsed * 80) % 360) - 40, 60, 40);
      };
      paint();
      const timer = setInterval(paint, 1000 / 15);
      const stream = canvas.captureStream(15);
      const audio = new AudioContext();
      await audio.resume().catch(() => {});
      const tone = audio.createOscillator();
      const quiet = audio.createGain();
      quiet.gain.value = 0.05;
      const output = audio.createMediaStreamDestination();
      tone.connect(quiet).connect(output);
      tone.start();
      for (const track of output.stream.getAudioTracks()) stream.addTrack(track);
      const recorder = new MediaRecorder(stream, {
        mimeType: "video/webm;codecs=vp8,opus",
        videoBitsPerSecond: 150_000,
        audioBitsPerSecond: 32_000,
      });
      const chunks: Blob[] = [];
      recorder.ondataavailable = (event) => chunks.push(event.data);
      const stopped = new Promise((resolve) => {
        recorder.onstop = resolve;
      });
      recorder.start();
      await new Promise((resolve) => setTimeout(resolve, length * 1000));
      recorder.stop();
      await stopped;
      clearInterval(timer);
      const bytes = new Uint8Array(await new Blob(chunks).arrayBuffer());
      let binary = "";
      for (let offset = 0; offset < bytes.length; offset += 0x8000)
        binary += String.fromCharCode(...bytes.subarray(offset, offset + 0x8000));
      return btoa(binary);
    }, seconds);
    return Buffer.from(base64, "base64");
  } finally {
    await page.close();
  }
}

/** The test video of this worker: see the `testVideo` fixture. */
let videoFile = "";

/**
 * Every worker makes one small vertical test video before its first test:
 * a moving test pattern with a tone from ffmpeg where that is installed,
 * otherwise a recording made by the browser.
 */
const test = baseTest.extend<object, { testVideo: string }>({
  testVideo: [
    async ({ browser }, use) => {
      const base = path.join(os.tmpdir(), `tokuly-e2e-short-${process.pid}`);
      try {
        execFileSync(
          "ffmpeg",
          [
            "-y",
            "-f",
            "lavfi",
            "-i",
            `testsrc2=size=180x320:rate=15:duration=${VIDEO_SECONDS}`,
            "-f",
            "lavfi",
            "-i",
            `sine=frequency=440:duration=${VIDEO_SECONDS}`,
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-crf",
            "36",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
            "-b:a",
            "48k",
            "-shortest",
            `${base}.mp4`,
          ],
          { stdio: "ignore" },
        );
        videoFile = `${base}.mp4`;
      } catch {
        fs.writeFileSync(`${base}.webm`, await recordVideo(browser, VIDEO_SECONDS));
        videoFile = `${base}.webm`;
      }
      await use(videoFile);
      fs.rmSync(videoFile, { force: true });
    },
    { scope: "worker", auto: true, timeout: 120_000 },
  ],
});

// These tests upload shorts and wait for the server to process them, and several watch for a while.
test.describe.configure({ timeout: 150_000 });

interface Channel {
  id: string;
  handle: string;
  name: string;
}

interface Short {
  id: string;
  shortId: string;
  status: string;
  sound?: { id: string };
}

function suffix(): string {
  return `${Date.now().toString(36).slice(-5)}${Math.random().toString(36).slice(2, 5)}`;
}

async function makeChannel(
  request: APIRequestContext,
  owner: Account,
  name: string,
): Promise<Channel> {
  const response = await rpc<{ channel: { summary: Channel } }>(
    request,
    `${CHANNELS}/CreateChannel`,
    { handle: `sh_${suffix()}`, name },
    owner,
  );
  return response.channel.summary;
}

/** Uploads the test video as a short of `owner` (public unless told otherwise) and waits until it is live. */
async function makeShort(
  request: APIRequestContext,
  owner: Account,
  caption: string,
  fields: Record<string, unknown> = {},
): Promise<Short> {
  const bytes = fs.readFileSync(videoFile);
  const upload = await rpc<{
    uploadId: string;
    putUrl: string;
    putHeaders?: Record<string, string>;
  }>(
    request,
    `${MEDIA}/CreateUpload`,
    {
      purpose: "ASSET_PURPOSE_SHORT",
      fileName: path.basename(videoFile),
      contentType: videoFile.endsWith(".webm") ? "video/webm" : "video/mp4",
      sizeBytes: String(bytes.length),
    },
    owner,
  );
  const put = await request.put(upload.putUrl, { data: bytes, headers: upload.putHeaders ?? {} });
  expect(put.ok(), "upload of the test video").toBeTruthy();
  const { asset } = await rpc<{ asset: { id: string } }>(
    request,
    `${MEDIA}/CompleteUpload`,
    { uploadId: upload.uploadId },
    owner,
  );
  const created = await rpc<{ short: Short }>(
    request,
    `${SHORTS}/CreateShort`,
    { assetId: asset.id, caption, visibility: "VISIBILITY_PUBLIC", ...fields },
    owner,
  );
  let short = created.short;
  await expect(async () => {
    short = (
      await rpc<{ short: Short }>(request, `${SHORTS}/GetShort`, { shortId: short.id }, owner)
    ).short;
    expect(short.status).toBe("SHORT_STATUS_PUBLISHED");
  }).toPass({ timeout: 90_000 });
  return short;
}

/** A creator with a channel, and a viewer who is their friend and subscribes to the channel. */
async function makeCreator(
  request: APIRequestContext,
  viewer?: Account,
): Promise<{ owner: Account; channel: Channel }> {
  const owner = await createAccount(request, "creator");
  const channel = await makeChannel(request, owner, `ダンス部${suffix()}`);
  if (viewer) {
    await befriend(request, owner, viewer);
    await rpc(request, `${CHANNELS}/Subscribe`, { channelId: channel.id }, viewer);
  }
  return { owner, channel };
}

/** A second browser signed in as `account`, with the same shape as the test's own. */
async function secondBrowser(
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
    hasTouch: (page.viewportSize()?.width ?? 1000) < 600,
  });
  const other = await context.newPage();
  await signInAs(other, account);
  return other;
}

// ───────────────────────────── The feed, from outside ─────────────────────────────

const isPhone = (page: Page): boolean => (page.viewportSize()?.width ?? 1000) < 600;

/** The slide of a short, wherever it is in the feed. */
const slideOf = (page: Page, short: Short): Locator =>
  page.locator(`[data-testid="short-slide"][data-short-id="${short.shortId}"]`);

/** The slide whose short is on screen. */
const currentSlide = (page: Page): Locator =>
  page.locator('[data-testid="short-slide"]:not([data-state="idle"])');

async function expectPlaying(page: Page, short: Short): Promise<void> {
  // The for-you feed brings a short round again where there are few: the visit on screen is the one.
  const slide = slideOf(page, short).and(currentSlide(page));
  await expect(slide).toHaveAttribute("data-state", "playing", { timeout: 30_000 });
  await expect
    .poll(
      () => slide.locator("video").evaluate((element: HTMLVideoElement) => element.currentTime),
      { timeout: 30_000 },
    )
    .toBeGreaterThan(0.2);
}

/** How many `<video>` elements the feed holds, and how many of them are playing. */
async function videos(page: Page): Promise<{ alive: number; playing: number }> {
  return page.evaluate(() => {
    const all = [...document.querySelectorAll("video")];
    return { alive: all.length, playing: all.filter((video) => !video.paused).length };
  });
}

/** Goes to the next short the way the device does: a swipe on a phone, the arrow key on a desktop. */
async function next(page: Page): Promise<void> {
  if (isPhone(page)) {
    // What a swipe does to the scroller; the snap points do the rest.
    await page
      .getByTestId("short-pager")
      .evaluate((element) => element.scrollBy({ top: element.clientHeight, behavior: "smooth" }));
  } else {
    await page.keyboard.press("ArrowDown");
  }
}

// ───────────────────────────── Tests ─────────────────────────────

test("a creator posts a vertical video with a caption; a subscribed friend finds it in their following feed", async ({
  page,
  me,
  request,
  browser,
  baseURL,
  testVideo,
}) => {
  const friend = await createAccount(request, "friend");
  const channel = await makeChannel(request, me, `ゆるダンス${suffix()}`);
  await befriend(request, me, friend);
  await rpc(request, `${CHANNELS}/Subscribe`, { channelId: channel.id }, friend);
  const tag = `bunkasai${suffix()}`;
  const caption = `文化祭のダンス! ${suffix()}`;

  await page.goto("/shorts/new");
  await page.getByTestId("short-file").setInputFiles(testVideo);
  await expect(page.getByTestId("short-preview")).toBeVisible();
  await expect(page.getByTestId("trim-summary")).toContainText("まるごと使うよ");

  // The caption: text, a hashtag, and a track by name.
  await page.getByLabel("キャプション").fill(`${caption} #${tag}`);
  await page.getByRole("button", { name: "曲名を書く" }).click();
  await page.getByLabel("曲名").fill("青春アンセム");
  await page.getByLabel("アーティスト").fill("Tokuly Band");

  // The cover is picked on the filmstrip.
  await page.getByRole("button", { name: "動画からえらぶ" }).click();
  await expect(page.getByTestId("cover-filmstrip")).toBeVisible({ timeout: 20_000 });
  await page.getByTestId("cover-filmstrip").getByRole("button").nth(3).click();

  await page.getByRole("button", { name: "投稿する" }).click();

  // The studio list follows the processing by itself: the short goes live without a reload.
  await expect(page).toHaveURL(/\/shorts\/studio$/, { timeout: 60_000 });
  const row = page.getByTestId("my-short").filter({ hasText: caption });
  await expect(row.getByTestId("my-short-status")).toHaveText("公開中", { timeout: 90_000 });
  await expect(row).toContainText("ともだちだけ");

  // The friend subscribes to the channel: the short is in their following feed and plays.
  const friendPage = await secondBrowser(browser, page, baseURL, friend);
  await friendPage.goto("/shorts?tab=following");
  const slide = currentSlide(friendPage);
  await expect(slide.getByTestId("short-caption")).toContainText(caption);
  await expect(slide).toHaveAttribute("data-state", "playing", { timeout: 30_000 });
  await expect(slide.getByTestId("short-sound")).toContainText("青春アンセム · Tokuly Band");
  await expect(slide.getByRole("link", { name: `#${tag}` })).toBeVisible();
  await friendPage.context().close();
});

test("swiping goes to the next short; only the one on screen plays and at most two videos are alive", async ({
  page,
  me,
  request,
}) => {
  const { owner } = await makeCreator(request, me);
  // Newest first: the feed shows `third`, `second`, `first`.
  const first = await makeShort(request, owner, `1本目 ${suffix()}`);
  const second = await makeShort(request, owner, `2本目 ${suffix()}`);
  const third = await makeShort(request, owner, `3本目 ${suffix()}`);

  await page.goto("/shorts?tab=following");
  await expectPlaying(page, third);
  expect(await videos(page)).toEqual({ alive: 2, playing: 1 });
  // The feed has the whole height of a phone: the tab bar is gone (the rail of wide screens stays).
  await expect(page.getByRole("navigation", { name: "main" })).toHaveCount(isPhone(page) ? 0 : 1);

  await next(page);
  await expectPlaying(page, second);
  await expect(slideOf(page, third)).toHaveAttribute("data-state", "idle");
  // The one left behind gave its video back; the next one is loading ahead.
  await expect(slideOf(page, third).locator("video")).toHaveCount(0);
  await expect(slideOf(page, first).locator("video")).toHaveCount(1);
  expect(await videos(page)).toEqual({ alive: 2, playing: 1 });

  await next(page);
  await expectPlaying(page, first);
  await expect.poll(() => videos(page)).toEqual({ alive: 1, playing: 1 });

  // A tap pauses, another one plays.
  const surface = slideOf(page, first).locator("[data-surface]");
  await surface.click();
  await expect(slideOf(page, first)).toHaveAttribute("data-state", "paused");
  await surface.click();
  await expect(slideOf(page, first)).toHaveAttribute("data-state", "playing");

  // After the last one, the feed says that this was all that is new.
  await next(page);
  await expect(page.getByText("新しいショートは全部見たよ! ✨")).toBeVisible();
  await expect.poll(async () => (await videos(page)).playing).toBe(0);

  // The sound switch is remembered.
  await page.getByTestId("short-mute").click();
  await expect(page.getByTestId("short-mute")).toHaveAttribute("aria-pressed", "true");
  await page.reload();
  await expect(page.getByTestId("short-mute")).toHaveAttribute("aria-pressed", "true");
});

test("on a desktop the keyboard drives the feed", async ({ page, me, request }) => {
  test.skip(isPhone(page), "keys belong to a desktop");
  const { owner } = await makeCreator(request, me);
  const first = await makeShort(request, owner, `キー1 ${suffix()}`);
  const second = await makeShort(request, owner, `キー2 ${suffix()}`);

  await page.goto("/shorts?tab=following");
  await expectPlaying(page, second);
  await page.keyboard.press("j");
  await expectPlaying(page, first);
  await page.keyboard.press("ArrowUp");
  await expectPlaying(page, second);
  await page.keyboard.press("Space");
  await expect(slideOf(page, second)).toHaveAttribute("data-state", "paused");
  await page.keyboard.press("Space");
  await expect(slideOf(page, second)).toHaveAttribute("data-state", "playing");
  await page.keyboard.press("m");
  await expect(page.getByTestId("short-mute")).toHaveAttribute("aria-pressed", "true");
  // The buttons next to the stage do the same for a mouse.
  await page.getByRole("button", { name: "次のショート" }).click();
  await expectPlaying(page, first);
});

test("a viewer likes with a double tap, saves and comments while the short keeps playing", async ({
  page,
  me: _me,
  request,
}) => {
  const { owner } = await makeCreator(request);
  const short = await makeShort(request, owner, `いいねしてね ${suffix()}`);
  const text = `かわいい! ${suffix()}`;

  await page.goto(`/shorts/${short.shortId}`);
  await expectPlaying(page, short);
  const slide = slideOf(page, short);

  // A double tap likes, with a heart where the finger was; a second double tap does not take it back.
  const like = slide.getByTestId("short-like");
  await expect(like).toHaveAttribute("aria-pressed", "false");
  await slide.locator("[data-surface]").dblclick({ position: { x: 120, y: 260 } });
  await expect(slide.getByTestId("like-burst")).toBeVisible();
  await expect(like).toHaveAttribute("aria-pressed", "true");
  await expect(like).toContainText("1");
  await slide.locator("[data-surface]").dblclick({ position: { x: 120, y: 260 } });
  await expect(like).toHaveAttribute("aria-pressed", "true");
  await expect(slide).toHaveAttribute("data-state", "playing");

  // Saved: the count moves and the short is on the saved page.
  const save = slide.getByTestId("short-save");
  await save.click();
  await expect(page.getByText("保存したよ! 🔖")).toBeVisible();
  await expect(save).toHaveAttribute("aria-pressed", "true");
  await expect(save).toContainText("1");

  // Comments open as a sheet over the short, which keeps playing.
  await slide.getByTestId("short-comments").click();
  const sheet = page.getByRole("dialog", { name: "コメント" });
  await sheet.getByLabel("コメントを書く…").fill(text);
  await sheet.getByRole("button", { name: "コメント", exact: true }).click();
  await expect(sheet.getByTestId("comment").filter({ hasText: text })).toBeVisible();
  await expect(slide).toHaveAttribute("data-state", "playing");
  await sheet.getByRole("button", { name: "close" }).click();
  await expect(slide.getByTestId("short-comments")).toContainText("1");

  await page.goto("/shorts/saved");
  await expect(page.getByTestId("short-tile")).toHaveCount(1);
  // Likes reach the liked list a moment later.
  await page.getByRole("tab", { name: "いいね" }).click();
  await expect(async () => {
    await page.reload();
    await expect(page.getByTestId("short-tile")).toHaveCount(1, { timeout: 8_000 });
  }).toPass({ timeout: 60_000 });
});

test("“not interested” takes a short out of the feed, and undo brings it back", async ({
  page,
  me,
  request,
}) => {
  const { owner } = await makeCreator(request, me);
  const older = await makeShort(request, owner, `のこる ${suffix()}`);
  const newer = await makeShort(request, owner, `きえる ${suffix()}`);

  await page.goto("/shorts?tab=following");
  await expectPlaying(page, newer);
  await slideOf(page, newer).getByRole("button", { name: "その他" }).click();
  await page.getByRole("menuitem", { name: "興味なし" }).click();
  await expect(page.getByText("おすすめに出にくくするね")).toBeVisible();
  await expect(slideOf(page, newer)).toHaveCount(0);
  await expectPlaying(page, older);

  // Taken back at once.
  await page.getByRole("button", { name: "元にもどす" }).click();
  await expect(slideOf(page, newer)).toHaveCount(1);

  // And for good this time: it stays away after a reload, also among what was seen before.
  await slideOf(page, newer).getByRole("button", { name: "その他" }).click();
  await page.getByRole("menuitem", { name: "興味なし" }).click();
  await expect(slideOf(page, newer)).toHaveCount(0);
  await page.reload();
  await expect(page.getByText("新しいショートは全部見たよ! ✨")).toBeVisible();
  await page.getByRole("button", { name: "見たショートももう一度" }).click();
  await expectPlaying(page, older);
  await expect(slideOf(page, newer)).toHaveCount(0);
});

test("a hashtag page lists the short, and a short opened from it swipes on through the tag", async ({
  page,
  me: _me,
  request,
}) => {
  const { owner } = await makeCreator(request);
  const tag = `neko${suffix()}`;
  const first = await makeShort(request, owner, `ねこ1 #${tag}`);
  const second = await makeShort(request, owner, `ねこ2 #${tag}`);

  await page.goto(`/shorts/tags/${tag}`);
  await expect(page.getByRole("heading", { name: `#${tag}` })).toBeVisible();
  await expect(page.getByTestId("hashtag-counts")).toContainText("ショート 2本");
  await page.getByRole("button", { name: "新着" }).click();
  const tiles = page.getByTestId("short-tile");
  await expect(tiles).toHaveCount(2);

  // The older one is second in "recent": it opens where it stands, with the newer one before it.
  await tiles.nth(1).click();
  await expect(page).toHaveURL(new RegExp(`/shorts/${first.shortId}\\?list=tag&key=${tag}`));
  await expectPlaying(page, first);
  await expect(page.getByRole("heading", { name: `#${tag}` })).toBeVisible();
  await expect(slideOf(page, second)).toHaveCount(1);

  // The hashtag in the caption leads back to the page.
  await slideOf(page, first)
    .getByRole("link", { name: `#${tag}` })
    .click();
  await expect(page).toHaveURL(new RegExp(`/shorts/tags/${tag}$`));
});

test("a short opened by its link starts on that short", async ({ page, me: _me, request }) => {
  const { owner, channel } = await makeCreator(request);
  await makeShort(request, owner, `まえ ${suffix()}`);
  const wanted = await makeShort(request, owner, `これ! ${suffix()}`);
  await makeShort(request, owner, `あと ${suffix()}`);

  await page.goto(`/shorts/${wanted.shortId}`);
  await expectPlaying(page, wanted);
  await expect(page.getByTestId("short-slide").first()).toHaveAttribute(
    "data-short-id",
    wanted.shortId,
  );
  await expect(
    currentSlide(page).getByRole("link", { name: channel.name, exact: true }),
  ).toBeVisible();

  // A short that does not exist says so and offers the feed.
  await page.goto("/shorts/zzzzzzzzzzz");
  await expect(page.getByText("このショートは見られないよ")).toBeVisible();
  await expect(page.getByRole("link", { name: "おすすめを見る" })).toBeVisible();
});

test("a sound page lists the shorts set to it and starts a new one with the sound", async ({
  page,
  me,
  request,
  testVideo,
}) => {
  const { owner } = await makeCreator(request);
  const title = `放課後ソング${suffix()}`;
  const short = await makeShort(request, owner, `音源つき ${suffix()}`, {
    sound: { external: { title, artist: "ゆるバンド" } },
  });
  await makeChannel(request, me, `わたしのch${suffix()}`);

  await page.goto(`/shorts/${short.shortId}`);
  await expectPlaying(page, short);
  await slideOf(page, short).getByTestId("short-sound").click();
  await expect(page).toHaveURL(new RegExp(`/sound/${short.sound?.id}$`));
  await expect(page.getByTestId("sound-title")).toHaveText(title);
  await expect(page.getByText("1本のショートで使われてるよ")).toBeVisible();
  await expect(page.getByTestId("short-tile")).toHaveCount(1);

  await page.getByRole("link", { name: "この音源を使う" }).click();
  await expect(page).toHaveURL(/\/shorts\/new\?sound=/);
  await page.getByTestId("short-file").setInputFiles(testVideo);
  await expect(page.getByTestId("picked-sound")).toContainText(`${title} · ゆるバンド`);
});

test("the creator edits, pins and deletes a short; it shows on the channel and the profile", async ({
  page,
  me,
  request,
}) => {
  const channel = await makeChannel(request, me, `マイch${suffix()}`);
  const caption = `さいしょ ${suffix()}`;
  const edited = `なおした ${suffix()}`;
  const short = await makeShort(request, me, caption);

  await page.goto("/shorts/studio");
  const row = page.getByTestId("my-short").filter({ hasText: caption });
  await expect(row.getByTestId("my-short-status")).toHaveText("公開中");
  await row.getByRole("button", { name: "このショートの操作" }).click();
  await page.getByRole("menuitem", { name: "チャンネルに固定" }).click();
  await expect(page.getByText("チャンネルに固定したよ! 📌")).toBeVisible();

  await row.getByRole("link", { name: "編集する" }).click();
  await expect(page.getByTestId("edit-status")).toHaveText("公開中だよ!");
  await page.getByLabel("キャプション").fill(edited);
  await page.getByRole("combobox", { name: "公開範囲" }).click();
  await page.getByRole("option", { name: /リンク限定/ }).click();
  await page.getByRole("button", { name: "保存する" }).click();
  await expect(page.getByText("保存したよ!", { exact: true })).toBeVisible();

  // Unlisted shorts are not on the channel; public ones are, pinned first.
  await rpc(
    request,
    `${SHORTS}/UpdateShort`,
    { shortId: short.id, visibility: "VISIBILITY_PUBLIC" },
    me,
  );
  await page.goto(`/c/${channel.handle}/shorts`);
  const tile = page.getByTestId("short-tile");
  await expect(tile).toHaveCount(1);
  await expect(tile).toContainText(edited);
  await expect(tile).toContainText("固定");

  // My own profile shows my shorts.
  await page.goto(`/u/${me.handle}`);
  await expect(page.getByTestId("profile-shorts").getByTestId("short-tile")).toHaveCount(1);

  await page.goto("/shorts/studio");
  await page.getByTestId("my-short").getByRole("button", { name: "このショートの操作" }).click();
  await page.getByRole("menuitem", { name: "削除する" }).click();
  await page.getByRole("dialog").getByRole("button", { name: "削除する" }).click();
  await expect(page.getByText("ショートを削除したよ")).toBeVisible();
  await expect(page.getByTestId("my-short")).toHaveCount(0);
});

test("analytics show a view after someone watched, and no watch numbers before there is enough data", async ({
  page,
  me,
  request,
  browser,
  baseURL,
}) => {
  await makeChannel(request, me, `数字ch${suffix()}`);
  const short = await makeShort(request, me, `見てね ${suffix()}`);
  const viewer = await createAccount(request, "viewer");

  // Someone watches it through its link, then leaves: that stay is reported.
  const viewerPage = await secondBrowser(browser, page, baseURL, viewer);
  await viewerPage.goto(`/shorts/${short.shortId}`);
  await expectPlaying(viewerPage, short);
  await viewerPage.waitForTimeout(2_000);
  await viewerPage.goto("/shorts/saved");
  await viewerPage.context().close();

  // Counts are live (the server writes them out every few seconds).
  await expect(async () => {
    await page.goto(`/shorts/studio/analytics?short=${short.shortId}&days=7`);
    await expect(page.getByTestId("analytics-views")).toHaveText("1", { timeout: 4_000 });
  }).toPass({ timeout: 60_000 });
  await expect(page.getByTestId("analytics-sources")).toContainText("シェアされたリンク");
  // How people watched is withheld for small audiences and the running day: not a zero.
  await expect(page.getByTestId("analytics-not-enough")).toHaveText("データがまだ足りないよ");

  // The channel's numbers include it and list it among the most viewed.
  await page.getByRole("button", { name: "チャンネル全体" }).click();
  await expect(page.getByTestId("analytics-views")).toHaveText("1");
  await expect(page.getByRole("heading", { name: "よく見られたショート" })).toBeVisible();

  // The studio list shows the public view count.
  await page.goto("/shorts/studio");
  await expect(page.getByTestId("my-short-views")).toHaveText("1");
});

test("after blocking a creator, their shorts are gone from the feed already loaded", async ({
  page,
  me,
  request,
}) => {
  const { owner } = await makeCreator(request, me);
  // The creator names themselves in the caption: the way from the feed to their profile.
  const short = await makeShort(request, owner, `ブロックされる @${owner.handle}`);

  await page.goto("/shorts?tab=following");
  await expectPlaying(page, short);

  // To their profile and block them, without leaving the app (the loaded feed stays in memory).
  await slideOf(page, short)
    .getByRole("link", { name: `@${owner.handle}` })
    .click();
  await expect(page).toHaveURL(new RegExp(`/u/${owner.handle}$`));
  page.once("dialog", (dialog) => void dialog.accept());
  await page.getByRole("button", { name: "ブロック" }).click();
  await expect(page).toHaveURL(/\/map$/);

  // Back to the feed through the history: what was loaded of theirs is gone.
  await page.goBack();
  await page.goBack();
  await expect(page).toHaveURL(/\/shorts\?tab=following$/);
  await expect(page.getByText("新しいショートは全部見たよ! ✨")).toBeVisible();
  await expect(slideOf(page, short)).toHaveCount(0);
});

test("friends-only and private shorts stay out of reach of others, by link and in feeds", async ({
  page,
  me,
  request,
}) => {
  const { owner, channel } = await makeCreator(request);
  // Subscribed, but not a friend.
  await rpc(request, `${CHANNELS}/Subscribe`, { channelId: channel.id }, me);
  const friendsOnly = await makeShort(request, owner, `ともだち限定 ${suffix()}`, {
    visibility: "VISIBILITY_FRIENDS",
  });
  const mine = await makeShort(request, owner, `自分だけ ${suffix()}`, {
    visibility: "VISIBILITY_PRIVATE",
  });

  for (const short of [friendsOnly, mine]) {
    await page.goto(`/shorts/${short.shortId}`);
    await expect(page.getByText("このショートは見られないよ")).toBeVisible();
    await expect(slideOf(page, short)).toHaveCount(0);
  }

  // Not in the following feed of a subscriber who is not a friend…
  await page.goto("/shorts?tab=following");
  await expect(page.getByText("新しいショートは全部見たよ! ✨")).toBeVisible();
  await page.getByRole("button", { name: "見たショートももう一度" }).click();
  await expect(page.getByText("まだショートがないよ")).toBeVisible();
  // …nor anywhere in their for-you feed.
  let cursor = "";
  for (let pageNumber = 0; pageNumber < 5; pageNumber++) {
    const feed = await rpc<{
      items?: { short?: { id: string } }[];
      page?: { nextCursor?: string };
    }>(
      request,
      "tokuly.shorts.v1.ShortFeedService/GetForYouFeed",
      { page: { limit: 30, cursor } },
      me,
    );
    const ids = (feed.items ?? []).map((item) => item.short?.id);
    expect(ids).not.toContain(friendsOnly.id);
    expect(ids).not.toContain(mine.id);
    cursor = feed.page?.nextCursor ?? "";
    if (!cursor) break;
  }

  // The creator's profile shows what is public, and opens it in the list of their shorts.
  const caption = `みんなに ${suffix()}`;
  const open = await makeShort(request, owner, caption);
  await page.goto(`/u/${owner.handle}`);
  const tiles = page.getByTestId("profile-shorts").getByTestId("short-tile");
  await expect(tiles).toHaveCount(1);
  await expect(tiles).toContainText(caption);
  await tiles.click();
  await expect(page).toHaveURL(new RegExp(`/shorts/${open.shortId}\\?.*list=user`));
});

test("posting can be cancelled, and a failed upload or post is tried again without uploading twice", async ({
  page,
  me,
  request,
  testVideo,
}) => {
  await makeChannel(request, me, `やりなおしch${suffix()}`);
  const caption = `やりなおし ${suffix()}`;
  let creates = 0;
  page.on("request", (sent) => {
    if (sent.url().includes("MediaService/CreateUpload")) creates += 1;
  });

  await page.goto("/shorts/new");
  await page.getByTestId("short-file").setInputFiles(testVideo);
  await page.getByLabel("キャプション").fill(caption);
  const post = page.getByRole("button", { name: "投稿する" });

  // 1. The upload hangs; cancelling stops it and nothing is posted.
  let stall = true;
  await page.route(
    (url) => !url.pathname.startsWith("/api/"),
    async (route) => {
      if (route.request().method() !== "PUT" || !stall) return route.fallback();
      await new Promise((resolve) => setTimeout(resolve, 8_000));
      await route.abort().catch(() => {});
    },
  );
  await post.click();
  await expect(page.getByTestId("post-status")).toContainText("アップロード中");
  await page.getByRole("button", { name: "やめる" }).click();
  await expect(page.getByText("投稿をやめたよ")).toBeVisible();
  await expect(post).toBeEnabled();

  // 2. The upload fails: it says so, and the next try uploads.
  stall = false;
  let failPut = true;
  await page.unrouteAll({ behavior: "ignoreErrors" });
  await page.route(
    (url) => !url.pathname.startsWith("/api/"),
    (route) => {
      if (route.request().method() !== "PUT" || !failPut) return route.fallback();
      failPut = false;
      return route.abort();
    },
  );
  await post.click();
  await expect(page.getByText("もう一度「投稿する」を押すと、続きからやり直すよ")).toBeVisible();
  await expect(post).toBeEnabled();

  // 3. The upload goes through but creating the short fails once.
  let failCreate = true;
  await page.route("**/tokuly.shorts.v1.ShortService/CreateShort", (route) => {
    if (!failCreate) return route.fallback();
    failCreate = false;
    return route.abort();
  });
  await post.click();
  await expect(
    page.getByText("もう一度「投稿する」を押すと、続きからやり直すよ").last(),
  ).toBeVisible();
  await expect(post).toBeEnabled();
  const uploadsSoFar = creates;

  // 4. Tried again: posted, with the video that is up already.
  await post.click();
  await expect(page).toHaveURL(/\/shorts\/studio$/, { timeout: 60_000 });
  await expect(page.getByTestId("my-short").filter({ hasText: caption })).toHaveCount(1);
  expect(creates).toBe(uploadsSoFar);
  const { shorts: listed = [] } = await rpc<{ shorts?: unknown[] }>(
    request,
    `${SHORTS}/ListMyShorts`,
    {},
    me,
  );
  expect(listed).toHaveLength(1);
});
