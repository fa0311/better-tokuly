import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import zlib from "node:zlib";
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
const VIDEOS = "tokuly.video.v1.VideoService";
const PLAYLISTS = "tokuly.video.v1.PlaylistService";
const PLAYBACK = "tokuly.video.v1.PlaybackService";
const MEDIA = "tokuly.media.v1.MediaService";

// ───────────────────────────── Arranging state ─────────────────────────────

/** Length of the test video in seconds. */
const VIDEO_SECONDS = 40;

/**
 * Records a moving test picture with a tone in the browser, in real time: the
 * way to a test video on machines without ffmpeg.
 */
async function recordVideo(browser: Browser, seconds: number): Promise<Buffer> {
  const page = await browser.newPage();
  try {
    const base64 = await page.evaluate(async (length) => {
      const canvas = document.createElement("canvas");
      canvas.width = 320;
      canvas.height = 180;
      const context = canvas.getContext("2d") as CanvasRenderingContext2D;
      const started = performance.now();
      const paint = () => {
        const elapsed = (performance.now() - started) / 1000;
        context.fillStyle = `hsl(${(elapsed * 40) % 360} 80% 50%)`;
        context.fillRect(0, 0, 320, 180);
        context.fillStyle = "#fff";
        context.fillRect(((elapsed * 60) % 360) - 40, 60, 40, 60);
        context.font = "24px sans-serif";
        context.fillText(elapsed.toFixed(1), 10, 30);
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
 * Every worker makes one test video before its first test: a moving test
 * pattern with a tone from ffmpeg where that is installed, otherwise a
 * recording made by the browser (which takes as long as the video is).
 */
const test = baseTest.extend<object, { testVideo: string }>({
  testVideo: [
    async ({ browser }, use) => {
      const base = path.join(os.tmpdir(), `tokuly-e2e-video-${process.pid}`);
      try {
        execFileSync(
          "ffmpeg",
          [
            "-y",
            "-f",
            "lavfi",
            "-i",
            `testsrc2=size=320x180:rate=15:duration=${VIDEO_SECONDS}`,
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
    { scope: "worker", auto: true, timeout: 180_000 },
  ],
});

// These tests upload videos and wait for the server to process them, and several watch for a while.
test.describe.configure({ timeout: 180_000 });

interface Channel {
  id: string;
  handle: string;
  name: string;
}

interface Video {
  id: string;
  shortId: string;
  title: string;
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
    { handle: `ch_${suffix()}`, name },
    owner,
  );
  return response.channel.summary;
}

/** Uploads the test video as `owner`, waits until it is processed, and (unless told not to) publishes it for everyone. */
async function makeVideo(
  request: APIRequestContext,
  owner: Account,
  title: string,
  options: { publish?: boolean; fields?: Record<string, unknown> } = {},
): Promise<Video> {
  const bytes = fs.readFileSync(videoFile);
  const contentType = videoFile.endsWith(".webm") ? "video/webm" : "video/mp4";
  const upload = await rpc<{
    uploadId: string;
    putUrl: string;
    putHeaders?: Record<string, string>;
  }>(
    request,
    `${MEDIA}/CreateUpload`,
    {
      purpose: "ASSET_PURPOSE_VIDEO",
      fileName: path.basename(videoFile),
      contentType,
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
  const created = await rpc<{ video: { summary: Video } }>(
    request,
    `${VIDEOS}/CreateVideo`,
    { assetId: asset.id, title, visibility: "VISIBILITY_PUBLIC", ...options.fields },
    owner,
  );
  const video = created.video.summary;
  await expect(async () => {
    const current = await rpc<{ video: { summary: { processing: string } } }>(
      request,
      `${VIDEOS}/GetVideo`,
      { videoId: video.id },
      owner,
    );
    expect(current.video.summary.processing).toBe("PROCESSING_STATE_READY");
  }).toPass({ timeout: 90_000 });
  if (options.publish !== false)
    await rpc(request, `${VIDEOS}/PublishVideo`, { videoId: video.id }, owner);
  return video;
}

/** A creator with a channel and one published video. */
async function makeCreator(
  request: APIRequestContext,
  title: string,
): Promise<{ owner: Account; channel: Channel; video: Video }> {
  const owner = await createAccount(request, "creator");
  const channel = await makeChannel(request, owner, `チャンネル${suffix()}`);
  const video = await makeVideo(request, owner, title);
  return { owner, channel, video };
}

/** A second browser signed in as `account`, with the same shape as the test's own. */
async function secondBrowser(
  browser: Browser,
  page: Page,
  baseURL: string | undefined,
  account?: Account,
): Promise<Page> {
  const context = await browser.newContext({
    baseURL,
    locale: "ja-JP",
    timezoneId: "Asia/Tokyo",
    viewport: page.viewportSize(),
    hasTouch: (page.viewportSize()?.width ?? 1000) < 600,
  });
  const other = await context.newPage();
  if (account) await signInAs(other, account);
  return other;
}

/** A small solid PNG picture, for channel art and thumbnails. */
function picture(name: string): { name: string; mimeType: string; buffer: Buffer } {
  const [width, height] = [320, 180];
  const row = width * 4 + 1;
  const raw = Buffer.alloc(row * height);
  for (let y = 0; y < height; y++)
    for (let x = 0; x < width; x++) raw.set([255, 92, 157, 255], y * row + 1 + x * 4);
  const chunk = (type: string, data: Buffer) => {
    const body = Buffer.concat([Buffer.from(type), data]);
    const out = Buffer.alloc(body.length + 8);
    out.writeUInt32BE(data.length, 0);
    body.copy(out, 4);
    out.writeUInt32BE(zlib.crc32(body) >>> 0, body.length + 4);
    return out;
  };
  const header = Buffer.alloc(13);
  header.writeUInt32BE(width, 0);
  header.writeUInt32BE(height, 4);
  header.set([8, 6, 0, 0, 0], 8);
  return {
    name,
    mimeType: "image/png",
    buffer: Buffer.concat([
      Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
      chunk("IHDR", header),
      chunk("IDAT", zlib.deflateSync(raw)),
      chunk("IEND", Buffer.alloc(0)),
    ]),
  };
}

// ───────────────────────────── The player, from outside ─────────────────────────────

const player = (page: Page): Locator => page.getByTestId("video-player");

async function currentTime(page: Page): Promise<number> {
  return page.locator("video").evaluate((element: HTMLVideoElement) => element.currentTime);
}

async function expectPlaying(page: Page): Promise<void> {
  await expect(player(page)).toHaveAttribute("data-state", "playing", { timeout: 30_000 });
  await expect.poll(() => currentTime(page), { timeout: 30_000 }).toBeGreaterThan(0.5);
}

// ───────────────────────────── Tests ─────────────────────────────

test("a new creator makes a channel, uploads a video and publishes it; a subscribed friend finds it", async ({
  page,
  me,
  request,
  browser,
  baseURL,
}) => {
  const friend = await createAccount(request, "friend");
  await befriend(request, me, friend);
  const handle = `yuru_${suffix()}`;
  const title = `はじめての動画 ${suffix()}`;

  // No channel yet: the studio explains the steps and leads to the channel form.
  await page.goto("/studio");
  await expect(page.getByRole("heading", { name: "クリエイターデビューしよう！" })).toBeVisible();
  await page.getByRole("link", { name: "チャンネルを作る" }).click();
  await page.getByLabel("チャンネル名").fill("ゆるっとゲーム部");
  await page.getByLabel("ハンドル（@からはじまるID）").fill(handle);
  await expect(page.getByText("このハンドルは使えます！")).toBeVisible();
  await page.getByLabel("紹介文").fill("毎日ゆるっとゲームしてるよ！");
  await page.getByRole("button", { name: "チャンネルを作る" }).click();
  await expect(page.getByText("チャンネルができたよ！🎉")).toBeVisible();
  await expect(page.getByTestId("studio-channel")).toContainText(`@${handle}`);

  // The friend subscribes before anything is published.
  await rpc(request, `${CHANNELS}/Subscribe`, { channelId: handle }, friend);

  // Upload: the file goes up while the title is typed.
  await page.getByRole("link", { name: "動画をアップロード" }).first().click();
  await page.getByTestId("video-file").setInputFiles(videoFile);
  await expect(page.getByTestId("upload-status")).toContainText("アップロード完了", {
    timeout: 60_000,
  });
  await page.getByLabel("タイトル").fill(title);
  await page.getByRole("button", { name: "つぎへ" }).click();

  // The editor follows processing; once it is done the video is published for everyone.
  await expect(page).toHaveURL(/\/studio\/videos\//);
  await expect(page.getByTestId("video-status")).toContainText("下書き");
  await expect(page.getByTestId("processing-card")).toHaveCount(0, { timeout: 90_000 });
  await expect(player(page)).toBeVisible();
  await page.getByTestId("video-status").getByRole("button", { name: "公開する" }).click();
  await page.getByRole("dialog").getByRole("button", { name: "いますぐ公開" }).click();
  await expect(page.getByText("動画を公開しました！🎉")).toBeVisible();
  await expect(page.getByTestId("video-status")).toContainText("公開中");

  // It is on the channel page…
  await page.goto(`/c/${handle}`);
  await expect(page.getByTestId("channel-name")).toHaveText("ゆるっとゲーム部");
  await expect(page.getByTestId("video-card").filter({ hasText: title }).first()).toBeVisible();

  // …and in the friend's subscriptions feed.
  const friendPage = await secondBrowser(browser, page, baseURL, friend);
  await friendPage.goto("/watch?feed=subscriptions");
  await expect(friendPage.getByTestId("video-card").filter({ hasText: title })).toBeVisible({
    timeout: 30_000,
  });
  await friendPage.getByRole("link", { name: title }).click();
  await expect(friendPage.getByTestId("video-title")).toHaveText(title);
  await friendPage.context().close();
});

test("the handle of a new channel is checked while it is typed", async ({ page, me, request }) => {
  const taken = await makeChannel(request, me, "先にあったチャンネル");
  await page.goto("/studio/channels/new");
  await page.getByLabel("チャンネル名").fill("あとからのチャンネル");
  await page.getByLabel("ハンドル（@からはじまるID）").fill(taken.handle);
  await expect(page.getByText("このハンドルはもう使われています")).toBeVisible();
  await expect(page.getByRole("button", { name: "チャンネルを作る" })).toBeDisabled();
  await page.getByLabel("ハンドル（@からはじまるID）").fill("a!");
  await expect(page.getByText("3〜24文字の a-z、0-9、_、- で入力してね")).toBeVisible();
});

test("a channel is edited — name, art, links — and deleted after typing its handle", async ({
  page,
  me,
  request,
}) => {
  const channel = await makeChannel(request, me, "編集まえのなまえ");
  await page.goto("/studio");
  await page.getByTestId("studio-channel").getByRole("link", { name: "編集する" }).click();
  await expect(page).toHaveURL(new RegExp(`/studio/channels/${channel.id}$`));

  await page.getByLabel("チャンネル名").fill("編集したなまえ");
  await page.getByLabel("アイコン", { exact: true }).setInputFiles(picture("icon.png"));
  await page.getByRole("button", { name: "リンクを追加" }).click();
  await page.getByLabel("なまえ", { exact: true }).fill("ブログ");
  await page.getByLabel("URL", { exact: true }).fill("https://example.com/blog");
  // Saving waits for the icon to finish uploading.
  await expect(page.getByRole("button", { name: "保存する" })).toBeEnabled({ timeout: 30_000 });
  await page.getByRole("button", { name: "保存する" }).click();
  await expect(page.getByText("チャンネルを保存しました")).toBeVisible();
  await expect(page.getByTestId("studio-channel")).toContainText("編集したなまえ");

  // The public page shows the new name, the icon and the link.
  await page.goto(`/c/${channel.handle}/about`);
  await expect(page.getByTestId("channel-name")).toHaveText("編集したなまえ");
  await expect(page.getByRole("link", { name: /ブログ/ })).toHaveAttribute(
    "href",
    "https://example.com/blog",
  );
  const { channel: stored } = await rpc<{ channel: { summary: { icon?: { url: string } } } }>(
    request,
    `${CHANNELS}/GetChannel`,
    { channelId: channel.id },
    me,
  );
  expect(stored.summary.icon?.url).toBeTruthy();

  // Deleting asks for the handle; without it the button stays off.
  await page.goto(`/studio/channels/${channel.id}`);
  await page.getByRole("button", { name: "チャンネルを削除する" }).click();
  const dialog = page.getByRole("dialog");
  await expect(dialog.getByRole("button", { name: "チャンネルを削除する" })).toBeDisabled();
  await dialog.getByRole("textbox").fill(channel.handle);
  await dialog.getByRole("button", { name: "チャンネルを削除する" }).click();
  await expect(page.getByText("チャンネルを削除しました")).toBeVisible();
  await expect(page.getByRole("heading", { name: "クリエイターデビューしよう！" })).toBeVisible();
});

test("the player starts by itself, resumes after a reload, and obeys the keyboard, the menu, thumbs and ?t=", async ({
  page,
  me: _me,
  request,
}, testInfo) => {
  const { video } = await makeCreator(request, `再生テスト ${suffix()}`);

  await page.goto(`/watch/${video.shortId}`);
  await expect(page.getByTestId("video-title")).toHaveText(video.title);
  await expectPlaying(page);

  // Watch for a bit, then pause with the keyboard: the position is reported.
  await expect.poll(() => currentTime(page), { timeout: 30_000 }).toBeGreaterThan(6);
  await page.keyboard.press("k");
  await expect(player(page)).toHaveAttribute("data-state", "paused");
  const stoppedAt = await currentTime(page);
  await expect(async () => {
    const { items } = await rpc<{
      items?: { video: { id: string; resumePositionSeconds?: number } }[];
    }>(request, `${PLAYBACK}/ListWatchHistory`, {}, _me);
    expect(
      items?.find((item) => item.video.id === video.id)?.video.resumePositionSeconds ?? 0,
    ).toBeGreaterThan(5);
  }).toPass({ timeout: 15_000 });

  // After a reload it picks up where it stopped.
  await page.reload();
  await expectPlaying(page);
  expect(await currentTime(page)).toBeGreaterThan(stoppedAt - 2);

  // Keyboard: j and l jump ten seconds, m mutes, digits jump to tenths.
  await page.keyboard.press("k");
  await expect(player(page)).toHaveAttribute("data-state", "paused");
  await page.keyboard.press("0");
  await expect.poll(() => currentTime(page)).toBeLessThan(1);
  await page.keyboard.press("l");
  await expect.poll(() => currentTime(page)).toBeGreaterThan(9);
  await page.keyboard.press("5");
  await expect.poll(() => currentTime(page)).toBeGreaterThan(19);
  await page.keyboard.press("j");
  await expect.poll(() => currentTime(page)).toBeLessThan(12);
  await page.keyboard.press("m");
  expect(await page.locator("video").evaluate((element: HTMLVideoElement) => element.muted)).toBe(
    true,
  );
  await page.keyboard.press("m");

  // The gear menu changes the speed.
  await player(page).getByRole("button", { name: "設定" }).click();
  const menu = page.getByTestId("player-settings");
  await menu.getByRole("button", { name: /再生速度/ }).click();
  await menu.getByRole("radio", { name: "1.5×" }).click();
  expect(
    await page.locator("video").evaluate((element: HTMLVideoElement) => element.playbackRate),
  ).toBe(1.5);
  await expect(menu).toHaveCount(0);

  if (testInfo.project.name === "phone") {
    // Thumbs: a double-tap on the right side of the picture jumps ten seconds ahead, on the left ten back.
    await page.keyboard.press("0");
    await expect.poll(() => currentTime(page)).toBeLessThan(1);
    const box = (await player(page).boundingBox()) as {
      x: number;
      y: number;
      width: number;
      height: number;
    };
    const doubleTap = async (fraction: number) => {
      await page.touchscreen.tap(box.x + box.width * fraction, box.y + box.height * 0.4);
      await page.touchscreen.tap(box.x + box.width * fraction, box.y + box.height * 0.4);
    };
    await doubleTap(0.85);
    await expect.poll(() => currentTime(page)).toBeGreaterThan(9);
    await doubleTap(0.85);
    await expect.poll(() => currentTime(page)).toBeGreaterThan(19);
    // (Two taps only count as a double-tap when they come quickly: on a busy machine, try again.)
    await expect(async () => {
      await doubleTap(0.15);
      expect(await currentTime(page)).toBeLessThan(12);
    }).toPass({ timeout: 20_000 });
  } else {
    // Wide screens: theatre mode stretches the player across the page, and is remembered.
    await page.keyboard.press("t");
    await expect(player(page).getByRole("button", { name: "シアターモードを終了" })).toBeVisible();
    await page.reload();
    await expect(player(page).getByRole("button", { name: "シアターモードを終了" })).toBeVisible();
    await page.keyboard.press("t");
    await expect(
      player(page).getByRole("button", { name: "シアターモード", exact: true }),
    ).toBeVisible();
  }

  // A deep link starts at its moment, whatever was watched before.
  await page.goto(`/watch/${video.shortId}?t=30`);
  await expectPlaying(page);
  expect(await currentTime(page)).toBeGreaterThanOrEqual(30);
});

test("chapters, a thumbnail and subtitles set in the studio show up for viewers", async ({
  page,
  me,
  request,
}) => {
  await makeChannel(request, me, `字幕チャンネル${suffix()}`);
  const video = await makeVideo(request, me, `字幕つき ${suffix()}`);

  await page.goto(`/studio/videos/${video.shortId}`);
  // Details: a tag and two chapters.
  await page.getByLabel("タグ").fill("ゲーム実況");
  await page.getByLabel("タグ").press("Enter");
  await page.getByRole("button", { name: "チャプターを追加" }).click();
  await page.getByLabel("チャプター1のなまえ").fill("オープニング");
  await page.getByRole("button", { name: "いまの再生位置に追加" }).click();
  // The second row landed at 0:00 too (the preview has not played): the form says what is wrong, and saving waits.
  await expect(page.getByTestId("chapter-row").nth(1).getByRole("alert")).toBeVisible();
  await page.getByLabel("チャプター2の開始時間").fill("0:20");
  await page.getByLabel("チャプター2のなまえ").fill("本編スタート");
  await page.getByRole("button", { name: "保存する" }).click();
  await expect(page.getByText("保存しました！")).toBeVisible();

  // Thumbnail: a moment picked from the video, then an uploaded picture, then back to the automatic one.
  await page.getByRole("button", { name: "動画からえらぶ" }).click();
  const picker = page.getByRole("dialog");
  await expect
    .poll(
      () => picker.locator("video").evaluate((element: HTMLVideoElement) => element.readyState),
      { timeout: 30_000 },
    )
    .toBeGreaterThanOrEqual(2);
  await picker.getByRole("button", { name: "この場面を使う" }).click();
  await expect(page.getByText("サムネイルを変更しました！")).toBeVisible({ timeout: 30_000 });
  await expect(picker).toHaveCount(0);
  await page.getByLabel("画像をアップロード").setInputFiles(picture("thumbnail.png"));
  await expect(page.getByText("サムネイルを変更しました！").last()).toBeVisible({
    timeout: 30_000,
  });
  await page.getByRole("button", { name: "自動に戻す" }).click();
  await expect(page.getByText("サムネイルを自動に戻しました")).toBeVisible();
  await expect(page.getByRole("button", { name: "自動に戻す" })).toHaveCount(0);

  // Subtitles: add a Japanese track by typing it.
  await page.getByRole("tab", { name: "字幕" }).click();
  await page.getByRole("button", { name: "字幕を追加" }).click();
  const dialog = page.getByRole("dialog");
  await dialog
    .getByLabel("字幕の内容")
    .fill("WEBVTT\n\n00:00:00.000 --> 00:00:30.000\nこんにちは、字幕です\n");
  await dialog.getByRole("button", { name: "字幕を追加" }).click();
  await expect(page.getByText("字幕を追加しました！")).toBeVisible();
  await expect(page.getByTestId("subtitle-track")).toContainText("日本語");
  await expect(page.getByTestId("subtitle-track")).toContainText("デフォルト");

  // On the watch page the default track is on, the tag is listed and the chapter is named.
  await page.goto(`/watch/${video.shortId}`);
  await expectPlaying(page);
  await expect(page.getByTestId("player-captions")).toHaveText("こんにちは、字幕です");
  await page.keyboard.press("c");
  await expect(page.getByTestId("player-captions")).toHaveCount(0);
  await page.getByRole("button", { name: "もっと見る" }).click();
  await expect(page.getByTestId("video-description")).toContainText("#ゲーム実況");
  await page
    .getByTestId("video-description")
    .getByRole("button", { name: /本編スタート/ })
    .click();
  await expect.poll(() => currentTime(page)).toBeGreaterThanOrEqual(20);
});

test("liking a video and saving it for later fill the built-in playlists", async ({
  page,
  me: _me,
  request,
}) => {
  const { video } = await makeCreator(request, `いいねテスト ${suffix()}`);
  await page.goto(`/watch/${video.shortId}`);
  await expect(page.getByTestId("like-count")).toHaveText("0");

  await page.getByRole("button", { name: "高評価" }).click();
  await expect(page.getByRole("button", { name: "高評価" })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  await expect(page.getByTestId("like-count")).toHaveText("1");
  await page.getByRole("button", { name: "あとで見る" }).click();
  await expect(page.getByText("「あとで見る」に追加しました！")).toBeVisible();

  // Both stay after a reload…
  await page.reload();
  await expect(page.getByRole("button", { name: "高評価" })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  await expect(page.getByRole("button", { name: "あとで見るに追加済み" })).toBeVisible();

  // …and the video is in "liked videos" and "watch later".
  await page.goto("/playlist/LL");
  await expect(page.getByTestId("playlist-title")).toHaveText("高評価した動画");
  await expect(page.getByTestId("video-card").filter({ hasText: video.title })).toBeVisible();
  await page.goto("/playlist/WL");
  await expect(page.getByTestId("video-card").filter({ hasText: video.title })).toBeVisible();

  // A dislike takes the like back.
  await page.goto(`/watch/${video.shortId}`);
  await page.getByRole("button", { name: "低評価" }).click();
  await expect(page.getByTestId("like-count")).toHaveText("0");
  await expect(page.getByRole("button", { name: "高評価" })).toHaveAttribute(
    "aria-pressed",
    "false",
  );
});

test("videos are saved to a new playlist, reordered and played through it", async ({
  page,
  me,
  request,
}) => {
  const creator = await makeCreator(request, `一本目 ${suffix()}`);
  const second = await makeVideo(request, creator.owner, `二本目 ${suffix()}`);
  const listName = `お気に入り ${suffix()}`;

  // Save the first video into a playlist made on the spot.
  await page.goto(`/watch/${creator.video.shortId}`);
  await page.getByRole("button", { name: "保存", exact: true }).click();
  const dialog = page.getByRole("dialog");
  await dialog.getByRole("button", { name: "新しい再生リスト" }).click();
  await dialog.getByLabel("タイトル").fill(listName);
  await dialog.getByRole("button", { name: "作成する" }).click();
  await expect(page.getByText(`「${listName}」に追加しました！`)).toBeVisible();

  // The second one goes in through the same dialog, from its card's menu.
  await page.goto(`/c/${creator.channel.handle}/videos`);
  await page.getByRole("button", { name: `「${second.title}」のメニュー` }).click();
  await page.getByRole("menuitem", { name: "再生リストに保存" }).click();
  await page
    .getByRole("dialog")
    .getByRole("checkbox", { name: new RegExp(listName) })
    .click();
  await expect(page.getByText(`「${listName}」に追加しました！`)).toBeVisible();
  await page.keyboard.press("Escape");

  // The library lists it; its page shows both videos in the order they were added.
  await page.goto("/watch/playlists");
  await page.getByTestId("playlist-card").filter({ hasText: listName }).click();
  await expect(page.getByTestId("playlist-title")).toHaveText(listName);
  const cards = page.getByTestId("video-card");
  await expect(cards).toHaveCount(2);
  await expect(cards.nth(0)).toContainText(creator.video.title);

  // Move the first video down with the keyboard; the order is kept by the server.
  await page.getByRole("button", { name: `「${creator.video.title}」を並べかえる` }).focus();
  await page.keyboard.press("ArrowDown");
  await expect(cards.nth(0)).toContainText(second.title);
  await expect(async () => {
    const { playlists } = await rpc<{ playlists: { id: string; title: string }[] }>(
      request,
      `${PLAYLISTS}/ListMyPlaylists`,
      {},
      me,
    );
    const list = playlists.find((playlist) => playlist.title === listName);
    const { items } = await rpc<{ items: { video: { id: string } }[] }>(
      request,
      `${PLAYLISTS}/ListPlaylistItems`,
      { playlistId: list?.id },
      me,
    );
    expect(items.map((item) => item.video.id)).toEqual([second.id, creator.video.id]);
  }).toPass({ timeout: 15_000 });

  // "Play all" starts the first video with the queue beside it.
  await page.getByRole("link", { name: "すべて再生" }).click();
  await expect(page.getByTestId("video-title")).toHaveText(second.title);
  await expect(page.getByTestId("playlist-queue")).toContainText("1 / 2");
  await expect(page.getByTestId("playlist-queue")).toContainText(creator.video.title);

  // Renaming and deleting the playlist.
  await page.goto("/watch/playlists");
  await page.getByTestId("playlist-card").filter({ hasText: listName }).click();
  await page.getByRole("button", { name: "編集する" }).click();
  await page.getByRole("dialog").getByLabel("タイトル").fill(`${listName} 改`);
  await page.getByRole("dialog").getByRole("button", { name: "保存する" }).click();
  await expect(page.getByTestId("playlist-title")).toHaveText(`${listName} 改`);
  await page.getByRole("button", { name: "削除する" }).click();
  await page.getByRole("dialog").getByRole("button", { name: "削除する" }).click();
  await expect(page).toHaveURL(/\/watch\/playlists$/);
  await expect(page.getByTestId("playlist-card").filter({ hasText: listName })).toHaveCount(0);
});

test("feeds: subscriptions filtered by genre, trending, and a recommendation turned away and brought back", async ({
  page,
  me,
  request,
}) => {
  const owner = await createAccount(request, "creator");
  const channel = await makeChannel(request, owner, `ジャンル${suffix()}`);
  const music = await makeVideo(request, owner, `音楽の動画 ${suffix()}`, {
    fields: { category: "VIDEO_CATEGORY_MUSIC" },
  });
  const gaming = await makeVideo(request, owner, `ゲームの動画 ${suffix()}`, {
    fields: { category: "VIDEO_CATEGORY_GAMING" },
  });
  await rpc(request, `${CHANNELS}/Subscribe`, { channelId: channel.id }, me);
  const card = (video: Video) => page.getByTestId("video-card").filter({ hasText: video.title });
  const genres = page.getByRole("group", { name: "ジャンル" });

  // The subscriptions feed has both videos; a genre chip narrows it down, and the address remembers it.
  await page.goto("/watch?feed=subscriptions");
  await expect(card(music)).toBeVisible();
  await expect(card(gaming)).toBeVisible();
  await genres.getByRole("button", { name: /音楽/ }).click();
  await expect(page).toHaveURL(/category=3/);
  await expect(card(music)).toBeVisible();
  await expect(card(gaming)).toHaveCount(0);
  await genres.getByRole("button", { name: /ペット/ }).click();
  await expect(page.getByText("このジャンルの動画は見つかりませんでした")).toBeVisible();
  await genres.getByRole("button", { name: "すべて" }).click();
  await expect(card(gaming)).toBeVisible();

  // Trending opens (a video only trends once it has been watched, so the list may still be empty).
  await page.getByRole("tab", { name: "急上昇" }).click();
  await expect(page).toHaveURL(/feed=trending/);
  await expect(
    page.getByTestId("video-card").first().or(page.getByText("急上昇中の動画はまだありません")),
  ).toBeVisible();

  // Home recommends the subscribed channel's videos. Turn one away, then take it back.
  await page.getByRole("tab", { name: "おすすめ" }).click();
  await expect(card(music)).toBeVisible();
  await card(music)
    .getByRole("button", { name: /のメニュー$/ })
    .click();
  await page.getByRole("menuitem", { name: "興味なし" }).click();
  await expect(page.getByTestId("video-dismissed")).toContainText("この動画はもうおすすめしません");
  await expect(card(music)).toHaveCount(0);
  await page.getByTestId("video-dismissed").getByRole("button", { name: "元に戻す" }).click();
  await expect(card(music)).toBeVisible();

  // Turning a whole channel away takes all of its videos along; that can be undone as well.
  await card(gaming)
    .getByRole("button", { name: /のメニュー$/ })
    .click();
  await page.getByRole("menuitem", { name: "このチャンネルをおすすめしない" }).click();
  await expect(page.getByTestId("video-dismissed")).toContainText(
    `「${channel.name}」の動画はもうおすすめしません`,
  );
  await expect(card(music)).toHaveCount(0);
  await expect(card(gaming)).toHaveCount(0);
  await page.getByTestId("video-dismissed").getByRole("button", { name: "元に戻す" }).click();
  await expect(card(music)).toBeVisible();
  await expect(card(gaming)).toBeVisible();
});

test("watch history keeps what was watched; entries can be removed and recording paused", async ({
  page,
  me,
  request,
}) => {
  const { video } = await makeCreator(request, `履歴テスト ${suffix()}`);
  await page.goto(`/watch/${video.shortId}`);
  await expectPlaying(page);
  await expect.poll(() => currentTime(page), { timeout: 30_000 }).toBeGreaterThan(3);
  await page.keyboard.press("k");

  await page.goto("/watch/history");
  await expect(page.getByRole("heading", { name: "今日" })).toBeVisible();
  const entry = page.getByTestId("video-card").filter({ hasText: video.title });
  await expect(entry).toBeVisible();

  await entry.getByRole("button", { name: /のメニュー$/ }).click();
  await page.getByRole("menuitem", { name: "履歴から削除" }).click();
  await expect(entry).toHaveCount(0);
  await expect(page.getByText("まだ履歴がないよ")).toBeVisible();

  // Pausing is a setting of the account.
  await page.getByRole("switch").click();
  await expect(page.getByText("履歴の記録を止めました")).toBeVisible();
  await expect(async () => {
    expect(
      (await rpc<{ paused?: boolean }>(request, `${PLAYBACK}/GetWatchHistorySettings`, {}, me))
        .paused,
    ).toBe(true);
  }).toPass();
  await expect(page.getByText("履歴の記録を止めています")).toBeVisible();
});

test("a viewer subscribes on the channel page and chooses notifications; the owner sees them", async ({
  page,
  me,
  request,
  browser,
  baseURL,
}) => {
  const { owner, channel, video } = await makeCreator(request, `チャンネル紹介 ${suffix()}`);
  await rpc(
    request,
    `${CHANNELS}/UpdateChannel`,
    {
      channelId: channel.id,
      description: "登録してね！",
      links: { links: [{ title: "ブログ", url: "https://example.com/blog" }] },
    },
    owner,
  );

  await page.goto(`/c/${channel.handle}`);
  await expect(page.getByTestId("channel-name")).toHaveText(channel.name);
  await expect(page.getByTestId("subscriber-count")).toHaveText("登録者 0人");
  await expect(
    page.getByTestId("video-card").filter({ hasText: video.title }).first(),
  ).toBeVisible();

  await page.getByRole("button", { name: "登録する" }).click();
  await expect(page.getByText(`「${channel.name}」を登録しました！`)).toBeVisible();
  await expect(page.getByTestId("subscriber-count")).toHaveText("登録者 1人");

  // Every upload, please.
  await page.getByRole("button", { name: `「${channel.name}」の登録と通知` }).click();
  await page.getByRole("menuitem", { name: "すべて通知" }).click();
  await expect(async () => {
    const { subscriptions } = await rpc<{
      subscriptions: { channel: { id: string }; notificationLevel: string }[];
    }>(request, `${CHANNELS}/ListSubscriptions`, {}, me);
    expect(
      subscriptions.find((subscription) => subscription.channel.id === channel.id)
        ?.notificationLevel,
    ).toBe("NOTIFICATION_LEVEL_ALL");
  }).toPass({ timeout: 15_000 });

  // The tabs: videos, about.
  await page.getByRole("tab", { name: "動画" }).click();
  await expect(page).toHaveURL(new RegExp(`/c/${channel.handle}/videos$`));
  await expect(page.getByTestId("video-card").filter({ hasText: video.title })).toBeVisible();
  await page.getByRole("tab", { name: "概要" }).click();
  await expect(page.getByText("登録してね！").last()).toBeVisible();
  await expect(page.getByRole("link", { name: /ブログ/ })).toHaveAttribute(
    "href",
    "https://example.com/blog",
  );

  // "My subscriptions" lists the channel.
  await page.goto("/watch/channels");
  await expect(
    page.getByTestId("channel-row").filter({ hasText: channel.name }).first(),
  ).toBeVisible();

  // The owner, and only the owner, has a subscribers tab with the new subscriber.
  await expect(page.getByRole("tab", { name: "登録者" })).toHaveCount(0);
  const ownerPage = await secondBrowser(browser, page, baseURL, owner);
  await ownerPage.goto(`/c/${channel.handle}`);
  await ownerPage.getByRole("tab", { name: "登録者" }).click();
  await expect(ownerPage.getByText(`@${me.handle}`)).toBeVisible();
  await ownerPage.context().close();

  // Unsubscribing.
  await page.goto(`/c/${channel.handle}`);
  await page.getByRole("button", { name: `「${channel.name}」の登録と通知` }).click();
  await page.getByRole("menuitem", { name: "登録を解除" }).click();
  await expect(page.getByRole("button", { name: "登録する" })).toBeVisible();
});

test("drafts and scheduled videos are the owner's alone; a video can be unpublished and deleted", async ({
  page,
  me,
  request,
  browser,
  baseURL,
}) => {
  const channel = await makeChannel(request, me, `予約チャンネル${suffix()}`);
  const draft = await makeVideo(request, me, `下書きの動画 ${suffix()}`, { publish: false });
  const visitor = await createAccount(request, "visitor");

  // The owner can open the watch page of a draft and is told what it is.
  await page.goto(`/watch/${draft.shortId}`);
  await expect(page.getByTestId("owner-notice")).toContainText("この動画はまだ下書きです");
  // Nobody else can.
  const visitorPage = await secondBrowser(browser, page, baseURL, visitor);
  await visitorPage.goto(`/watch/${draft.shortId}`);
  await expect(visitorPage.getByText("この動画は見られません")).toBeVisible();

  // Schedule it from the editor.
  await page.goto(`/studio/videos/${draft.shortId}`);
  await page.getByTestId("video-status").getByRole("button", { name: "公開する" }).click();
  const dialog = page.getByRole("dialog");
  await dialog.getByRole("tab", { name: "日時を予約" }).click();
  await dialog.getByRole("button", { name: "予約する" }).click();
  await expect(page.getByText("公開を予約しました！")).toBeVisible();
  await expect(page.getByTestId("video-status")).toContainText("に公開");
  await page.goto("/studio");
  await page.getByRole("tab", { name: "公開予約" }).click();
  await expect(page.getByTestId("studio-video").filter({ hasText: draft.title })).toBeVisible();

  // Cancel the schedule, publish now, and the visitor can watch.
  await page.goto(`/studio/videos/${draft.shortId}`);
  await page.getByRole("button", { name: "予約を取り消す" }).click();
  await expect(page.getByText("下書きに戻しました")).toBeVisible();
  await page.getByTestId("video-status").getByRole("button", { name: "公開する" }).click();
  await page.getByRole("dialog").getByRole("button", { name: "いますぐ公開" }).click();
  await expect(page.getByText("動画を公開しました！🎉")).toBeVisible();
  await visitorPage.goto(`/watch/${draft.shortId}`);
  await expect(visitorPage.getByTestId("video-title")).toHaveText(draft.title);
  await visitorPage.context().close();

  // Delete it from the studio's list.
  await page.goto("/studio");
  const row = page.getByTestId("studio-video").filter({ hasText: draft.title });
  await row.getByRole("button", { name: /のメニュー$/ }).click();
  await page.getByRole("menuitem", { name: "削除する" }).click();
  await page.getByRole("dialog").getByRole("button", { name: "削除する" }).click();
  await expect(page.getByText("動画を削除しました")).toBeVisible();
  await expect(row).toHaveCount(0);
  await page.goto(`/c/${channel.handle}/videos`);
  await expect(page.getByText("動画はまだありません")).toBeVisible();
});

test("studio analytics show a channel's and a video's numbers", async ({ page, me, request }) => {
  const channel = await makeChannel(request, me, `分析チャンネル${suffix()}`);
  const video = await makeVideo(request, me, `分析する動画 ${suffix()}`);
  // Someone watches, so there is something to count.
  const viewer = await createAccount(request, "viewer");
  const { sessionId } = await rpc<{ sessionId: string }>(
    request,
    `${PLAYBACK}/StartPlayback`,
    { videoId: video.id, source: "TRAFFIC_SOURCE_SEARCH" },
    viewer,
  );
  await rpc(
    request,
    `${PLAYBACK}/ReportPlayback`,
    { sessionId, positionSeconds: 12, playing: true },
    viewer,
  );
  await rpc(
    request,
    `${CHANNELS}/Subscribe`,
    { channelId: channel.id, sourceVideoId: video.id },
    viewer,
  );

  await page.goto("/studio/analytics");
  await expect(page.getByRole("heading", { name: "アナリティクス" })).toBeVisible();
  await expect(page.getByTestId("realtime-analytics")).toBeVisible();
  await expect(page.getByTestId("stat-tile")).toHaveCount(4);
  await expect(page.getByTestId("stat-tile").filter({ hasText: "視聴回数" })).toBeVisible();
  await expect(page.getByTestId("trend-chart")).toBeVisible();
  await expect(page.getByRole("region", { name: "どこから見に来た？" })).toBeVisible();
  // The period is the one filter: it scopes every number below it.
  await page.getByRole("tab", { name: "過去7日" }).click();
  await expect(page.getByRole("tab", { name: "過去7日" })).toHaveAttribute("aria-selected", "true");
  await expect(page.getByTestId("trend-chart")).toBeVisible();
  // The chart can be read with the keyboard: its value text names a day and a number.
  await page.getByTestId("trend-chart").focus();
  await page.keyboard.press("ArrowLeft");
  await expect(page.getByTestId("trend-chart")).toHaveAttribute("aria-valuetext", /.+: .+/);

  // One video: the same report plus audience retention.
  await page.goto(`/studio/videos/${video.shortId}?tab=analytics`);
  await expect(page.getByRole("region", { name: "視聴者維持率" })).toBeVisible();
  await expect(page.getByTestId("stat-tile")).toHaveCount(4);
  await expect(page.getByTestId("realtime-analytics")).toBeVisible();
});

test("a watch party: playback follows the host, people chat, a guest joins by link", async ({
  page,
  me,
  request,
  browser,
  baseURL,
}) => {
  const { owner, video } = await makeCreator(request, `パーティーの動画 ${suffix()}`);
  const nextVideo = await makeVideo(request, owner, `つぎの動画 ${suffix()}`);
  const friend = await createAccount(request, "friend");
  await befriend(request, me, friend);

  // The host starts a party from the watch page.
  await page.goto(`/watch/${video.shortId}`);
  await expectPlaying(page);
  await page.getByRole("button", { name: "一緒に見る" }).click();
  await page.getByRole("dialog").getByRole("button", { name: "パーティーをはじめる" }).click();
  await expect(page).toHaveURL(/\/party\//);
  await expect(page.getByTestId("party-room")).toBeVisible();
  const code = new URL(page.url()).pathname.split("/").pop() as string;

  // The invite shows the link that works for everyone, and notifies friends.
  await page.getByRole("button", { name: "招待" }).click();
  await expect(page.getByTestId("party-link")).toHaveText(new RegExp(`/join/party/${code}$`));
  await page
    .getByRole("dialog")
    .getByRole("checkbox", { name: new RegExp(friend.handle) })
    .click();
  await page.getByRole("dialog").getByRole("button", { name: "1人に招待を送る" }).click();
  await expect(page.getByText("1人に招待を送りました！")).toBeVisible();
  await page.keyboard.press("Escape");

  // A friend opens the link, sees what is on, and joins.
  const friendPage = await secondBrowser(browser, page, baseURL, friend);
  await friendPage.goto(`/join/party/${code}`);
  await expect(friendPage).toHaveURL(new RegExp(`/party/${code}$`));
  await expect(friendPage.getByTestId("party-preview")).toContainText(video.title);
  await friendPage.getByRole("button", { name: "参加する" }).click();
  await expect(friendPage.getByTestId("party-room")).toBeVisible();
  await expect(page.getByTestId("party-count")).toHaveText("2人が参加中");
  // Only the host drives: the friend's player says so.
  await expect(friendPage.getByText("ホストが再生を操作しています")).toBeVisible();

  // The host plays: the friend's player starts and stays close.
  await expect(player(page)).toBeVisible();
  await page.keyboard.press("k");
  await expectPlaying(page);
  await expectPlaying(friendPage);
  await expect
    .poll(async () => Math.abs((await currentTime(page)) - (await currentTime(friendPage))), {
      timeout: 20_000,
    })
    .toBeLessThan(2.5);

  // The host jumps ahead: the friend follows.
  await page.keyboard.press("5");
  await expect.poll(() => currentTime(page)).toBeGreaterThan(19);
  await expect.poll(() => currentTime(friendPage), { timeout: 20_000 }).toBeGreaterThan(19);

  // The host pauses: so does the friend, on the same spot.
  await page.keyboard.press("k");
  await expect(player(page)).toHaveAttribute("data-state", "paused");
  await expect(player(friendPage)).toHaveAttribute("data-state", "paused", { timeout: 20_000 });
  await expect
    .poll(async () => Math.abs((await currentTime(page)) - (await currentTime(friendPage))), {
      timeout: 20_000,
    })
    .toBeLessThan(1);

  // Chat goes both ways.
  await friendPage.getByLabel("メッセージを送る…").fill("やっほー！");
  await friendPage.getByRole("button", { name: "送信" }).click();
  await expect(page.getByTestId("party-message").filter({ hasText: "やっほー！" })).toBeVisible();
  await page.getByLabel("メッセージを送る…").fill("いらっしゃい！");
  await page.getByLabel("メッセージを送る…").press("Enter");
  await expect(
    friendPage.getByTestId("party-message").filter({ hasText: "いらっしゃい！" }),
  ).toBeVisible();

  // The host puts another video on, by its link: everyone gets it.
  await page.getByRole("button", { name: "動画を変える" }).click();
  const picker = page.getByRole("dialog");
  await picker.getByRole("tab", { name: "リンクから" }).click();
  await picker
    .getByLabel("動画のリンクかIDを貼りつけ")
    .fill(`https://tokuly.example/watch/${nextVideo.shortId}`);
  await picker
    .getByTestId("party-pick")
    .filter({ hasText: nextVideo.title })
    .getByRole("button", { name: "これにする" })
    .click();
  await expect(page.getByTestId("party-room")).toContainText(nextVideo.title);
  await expect(friendPage.getByTestId("party-room")).toContainText(nextVideo.title);
  await expect(player(friendPage)).toHaveAttribute("data-state", "paused");

  // Someone without an account joins as a guest with just a name.
  const guestPage = await secondBrowser(browser, page, baseURL);
  await guestPage.goto(`/join/party/${code}`);
  await expect(guestPage.getByTestId("party-preview")).toContainText(nextVideo.title);
  await guestPage.getByLabel("なまえ").fill("とおりすがり");
  await guestPage.getByRole("button", { name: "ゲストで参加する" }).click();
  await expect(guestPage.getByTestId("party-room")).toBeVisible();
  await expect(page.getByTestId("party-count")).toHaveText("3人が参加中");
  await expect(page.getByTestId("party-chat")).toContainText("とおりすがりさんが参加しました");
  await expect(player(guestPage)).toBeVisible();

  // The member list names everyone with their role; the host removes the guest.
  await page.getByRole("tab", { name: /メンバー/ }).click();
  await expect(page.getByTestId("party-member")).toHaveCount(3);
  await expect(page.getByTestId("party-member").first()).toContainText("ホスト");
  await page.getByRole("button", { name: "とおりすがりさんのメニュー" }).click();
  await page.getByRole("menuitem", { name: "退出させる" }).click();
  await page.getByRole("dialog").getByRole("button", { name: "退出させる" }).click();
  await expect(guestPage.getByTestId("party-farewell")).toContainText(
    "ホストがあなたを退出させました",
  );
  await expect(page.getByTestId("party-member")).toHaveCount(2);
  await guestPage.context().close();

  // The host ends the party: it is over for the friend too.
  await page.getByRole("button", { name: "パーティーのメニュー" }).click();
  await page.getByRole("menuitem", { name: "パーティーを終了する" }).click();
  await page.getByRole("dialog").getByRole("button", { name: "パーティーを終了する" }).click();
  await expect(page.getByTestId("party-farewell")).toContainText("パーティーは終了しました");
  await expect(friendPage.getByTestId("party-farewell")).toContainText(
    "ホストがパーティーを終了しました",
  );
  await friendPage.context().close();
});

test("the hub leads to the studio, the history and the playlists", async ({ page, me: _me }) => {
  await page.goto("/me");
  for (const [label, href] of [
    ["スタジオ", "/studio"],
    ["再生履歴", "/watch/history"],
    ["再生リスト", "/watch/playlists"],
  ] as const) {
    await expect(page.getByRole("main").getByRole("link", { name: label })).toHaveAttribute(
      "href",
      href,
    );
  }
});
