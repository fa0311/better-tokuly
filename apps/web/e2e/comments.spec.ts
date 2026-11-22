import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import zlib from "node:zlib";
import type { APIRequestContext, Browser, Locator, Page } from "@playwright/test";
import { type Account, test as baseTest, createAccount, expect, rpc, signInAs } from "./support";

const CHANNELS = "tokuly.video.v1.ChannelService";
const VIDEOS = "tokuly.video.v1.VideoService";
const MEDIA = "tokuly.media.v1.MediaService";
const COMMENTS = "tokuly.engagement.v1.CommentService";
const MODERATION = "tokuly.engagement.v1.CommentModerationService";

// ───────────────────────────── Arranging state ─────────────────────────────

/** Length of the test video in seconds. */
const VIDEO_SECONDS = 24;

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
      const base = path.join(os.tmpdir(), `tokuly-e2e-comments-${process.pid}`);
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

// Each test uploads a video to comment on and waits for the server to process it.
test.describe.configure({ timeout: 150_000 });

function suffix(): string {
  return `${Date.now().toString(36).slice(-5)}${Math.random().toString(36).slice(2, 5)}`;
}

interface Creator {
  owner: Account;
  channel: { id: string; handle: string; name: string };
  video: { id: string; shortId: string; title: string };
}

/** A creator with a channel and one published video for people to comment on. */
async function makeCreator(request: APIRequestContext): Promise<Creator> {
  const owner = await createAccount(request, "creator");
  const { channel } = await rpc<{ channel: { summary: Creator["channel"] } }>(
    request,
    `${CHANNELS}/CreateChannel`,
    { handle: `ch_${suffix()}`, name: `コメント部${suffix()}` },
    owner,
  );
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
  const created = await rpc<{ video: { summary: Creator["video"] } }>(
    request,
    `${VIDEOS}/CreateVideo`,
    { assetId: asset.id, title: `コメントの動画 ${suffix()}`, visibility: "VISIBILITY_PUBLIC" },
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
  await rpc(request, `${VIDEOS}/PublishVideo`, { videoId: video.id }, owner);
  return { owner, channel: channel.summary, video };
}

/** Writes a comment through the API, as `author`. */
async function writeComment(
  request: APIRequestContext,
  author: Account,
  videoId: string,
  text: string,
  fields: Record<string, unknown> = {},
): Promise<{ id: string; status: string }> {
  const { comment } = await rpc<{ comment: { id: string; status: string } }>(
    request,
    `${COMMENTS}/CreateComment`,
    { subject: { type: "SUBJECT_TYPE_VIDEO", id: videoId }, text, ...fields },
    author,
  );
  return comment;
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

/** A small solid PNG picture. */
function picture(): { name: string; mimeType: string; buffer: Buffer } {
  const size = 64;
  const row = size * 4 + 1;
  const raw = Buffer.alloc(row * size);
  for (let y = 0; y < size; y++)
    for (let x = 0; x < size; x++) raw.set([124, 92, 255, 255], y * row + 1 + x * 4);
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
  return {
    name: "picture.png",
    mimeType: "image/png",
    buffer: Buffer.concat([
      Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
      chunk("IHDR", header),
      chunk("IDAT", zlib.deflateSync(raw)),
      chunk("IEND", Buffer.alloc(0)),
    ]),
  };
}

// ───────────────────────────── The thread, from outside ─────────────────────────────

const thread = (page: Page): Locator => page.getByTestId("comment-thread");
const commentWith = (page: Page, text: string): Locator =>
  thread(page).getByTestId("comment").filter({ hasText: text });

/** Opens the watch page and stops the video, so nothing moves while the thread is used. */
async function openThread(page: Page, shortId: string): Promise<void> {
  await page.goto(`/watch/${shortId}`);
  await expect(thread(page).getByTestId("comment-count")).toBeVisible({ timeout: 30_000 });
}

async function post(page: Page, text: string): Promise<void> {
  await thread(page).getByLabel("コメントを書く…").fill(text);
  await thread(page).getByRole("button", { name: "コメント", exact: true }).click();
}

// ───────────────────────────── Tests ─────────────────────────────

test("a viewer comments; the owner replies, pins and hearts it; the viewer deletes it — all live", async ({
  page,
  me: _me,
  request,
  browser,
  baseURL,
}) => {
  const { owner, video } = await makeCreator(request);
  const text = `最高でした！ ${suffix()}`;
  await openThread(page, video.shortId);
  await expect(thread(page).getByText("まだコメントがないよ")).toBeVisible();

  const ownerPage = await secondBrowser(browser, page, baseURL, owner);
  await openThread(ownerPage, video.shortId);

  // The comment shows at once for its author and arrives live for the owner.
  await post(page, text);
  await expect(commentWith(page, text)).toBeVisible();
  await expect(thread(page).getByTestId("comment-count")).toHaveText("1件");
  await expect(commentWith(ownerPage, text)).toBeVisible();

  // The owner answers (as the channel the video is on, which earns the creator badge).
  await commentWith(ownerPage, text).getByRole("button", { name: "返信", exact: true }).click();
  await thread(ownerPage).getByLabel("返信を書く…").fill("見てくれてありがとう！");
  await thread(ownerPage).getByRole("button", { name: "返信", exact: true }).last().click();
  const reply = thread(ownerPage)
    .getByTestId("comment")
    .filter({ hasText: "見てくれてありがとう！" })
    .last();
  await expect(reply).toBeVisible();
  await expect(reply.getByText("投稿者", { exact: true })).toBeVisible();

  // The viewer sees the reply count grow and opens the reply.
  await commentWith(page, text).getByRole("button", { name: "返信 1件" }).click();
  await expect(thread(page).getByText("見てくれてありがとう！")).toBeVisible();
  await expect(thread(page).getByTestId("comment-count")).toHaveText("2件");

  // Pin and heart, as the owner; both show for the viewer without a reload.
  await commentWith(ownerPage, text)
    .first()
    .getByRole("button", { name: "その他" })
    .first()
    .click();
  await ownerPage.getByRole("menuitem", { name: "固定する" }).click();
  await expect(ownerPage.getByText("コメントを固定したよ！")).toBeVisible();
  await expect(commentWith(page, text).first().getByText("固定されたコメント")).toBeVisible();
  await commentWith(ownerPage, text)
    .first()
    .getByRole("button", { name: "ハートをつける" })
    .first()
    .click();
  await expect(
    commentWith(page, text).first().getByRole("img", { name: "投稿者がハートをつけました" }),
  ).toBeVisible();

  // The viewer likes the reply and deletes their own comment; the thread is empty again for both.
  await thread(page)
    .getByTestId("comment")
    .filter({ hasText: "見てくれてありがとう！" })
    .last()
    .getByRole("button", { name: "いいね" })
    .click();
  await expect(
    thread(page)
      .getByTestId("comment")
      .filter({ hasText: "見てくれてありがとう！" })
      .last()
      .getByRole("button", { name: "いいね" }),
  ).toHaveAttribute("aria-pressed", "true");
  await commentWith(page, text).first().getByRole("button", { name: "その他" }).first().click();
  await page.getByRole("menuitem", { name: "削除する" }).click();
  await expect(page.getByRole("dialog")).toContainText("ついている返信 1件もいっしょに消えます");
  await page.getByRole("dialog").getByRole("button", { name: "削除する" }).click();
  await expect(commentWith(page, text)).toHaveCount(0);
  await expect(thread(page).getByTestId("comment-count")).toHaveText("0件");
  await expect(commentWith(ownerPage, text)).toHaveCount(0);
  await ownerPage.context().close();
});

test("comments held for review wait in the owner's queue until approved", async ({
  page,
  me,
  request,
  browser,
  baseURL,
}) => {
  const { owner, video } = await makeCreator(request);
  const viewer = await createAccount(request, "viewer");
  const text = `承認まちのコメント ${suffix()}`;

  // The owner (this browser) turns on "review every comment" in the comment manager.
  await signInAs(page, owner);
  await page.goto("/settings/comments?tab=rules");
  await page.getByRole("switch", { name: /すべてのコメントを確認する/ }).click();
  await expect(async () => {
    const { settings } = await rpc<{ settings: { holdAll?: boolean } }>(
      request,
      `${MODERATION}/GetModerationSettings`,
      {},
      owner,
    );
    expect(settings.holdAll).toBe(true);
  }).toPass({ timeout: 15_000 });

  // A viewer comments: they are told it waits, and nobody else sees it.
  const viewerPage = await secondBrowser(browser, page, baseURL, viewer);
  await openThread(viewerPage, video.shortId);
  await post(viewerPage, text);
  await expect(
    viewerPage.getByText("コメントを送ったよ！投稿者が確認したら表示されます"),
  ).toBeVisible();
  await expect(commentWith(viewerPage, text).getByText("確認待ち", { exact: true })).toBeVisible();
  await expect(thread(viewerPage).getByTestId("comment-count")).toHaveText("0件");
  const comments = await rpc<{ comments?: unknown[] }>(
    request,
    `${COMMENTS}/ListComments`,
    { subject: { type: "SUBJECT_TYPE_VIDEO", id: video.id } },
    me,
  );
  expect(comments.comments ?? []).toHaveLength(0);

  // It is in the owner's queue, with the reason; approving publishes it.
  await page.goto("/settings/comments");
  const row = page.getByTestId("comment-row").filter({ hasText: text });
  await expect(row).toBeVisible();
  await expect(row).toContainText("すべてのコメントを確認する設定");
  await expect(row).toContainText(video.title);
  await expect(page.getByRole("tab", { name: /確認待ち/ })).toContainText("1");
  await row.getByRole("button", { name: "承認する" }).click();
  await expect(page.getByText("コメントを承認したよ！")).toBeVisible();
  await expect(row).toHaveCount(0);
  await expect(page.getByText("確認待ちのコメントはありません")).toBeVisible();

  // The viewer sees their comment go public without reloading.
  await expect(commentWith(viewerPage, text).getByText("確認待ち", { exact: true })).toHaveCount(0);
  await expect(thread(viewerPage).getByTestId("comment-count")).toHaveText("1件");
  // It is listed under "received", published; hiding it takes it out of public view again.
  await page.getByRole("tab", { name: "届いたコメント" }).click();
  const received = page.getByTestId("comment-row").filter({ hasText: text });
  await received.getByRole("button", { name: "非表示にする" }).click();
  await expect(received.getByText("非表示", { exact: true })).toBeVisible();
  await expect(commentWith(viewerPage, text).getByText("非表示", { exact: true })).toBeVisible();
  await viewerPage.context().close();
});

test("blocked words send a comment to review; a blocked commenter can be unblocked", async ({
  page,
  me: _me,
  request,
}) => {
  const { owner, video } = await makeCreator(request);
  const troll = await createAccount(request, "troll");
  await signInAs(page, owner);

  // Add a blocked word in the rules.
  await page.goto("/settings/comments?tab=rules");
  await page.getByLabel(/言葉を入力/).fill("ネタバレ");
  await page.getByRole("button", { name: "追加" }).click();
  await expect(page.getByRole("list", { name: "ブロックする言葉" })).toContainText("ネタバレ");
  await expect(async () => {
    const { settings } = await rpc<{ settings: { blockedWords?: string[] } }>(
      request,
      `${MODERATION}/GetModerationSettings`,
      {},
      owner,
    );
    expect(settings.blockedWords).toEqual(["ネタバレ"]);
  }).toPass({ timeout: 15_000 });

  // A comment with the word is held; one without is published.
  const held = await writeComment(request, troll, video.id, `ネタバレするね ${suffix()}`);
  expect(held.status).toBe("COMMENT_STATUS_HELD");
  expect(
    (await writeComment(request, troll, video.id, `ふつうのコメント ${suffix()}`)).status,
  ).toBe("COMMENT_STATUS_PUBLISHED");

  // The queue shows why; the owner blocks its author from there.
  await page.getByRole("tab", { name: /確認待ち/ }).click();
  const row = page.getByTestId("comment-row").filter({ hasText: "ネタバレするね" });
  await expect(row).toContainText("ブロックした言葉が入っています");
  await row.getByRole("button", { name: "その他" }).click();
  await page.getByRole("menuitem", { name: "コメントをブロック" }).click();
  await page.getByRole("dialog").getByRole("button", { name: "コメントをブロック" }).click();
  await expect(page.getByText("trollさんのコメントをブロックしました")).toBeVisible();
  await expect(row).toHaveCount(0);

  // They are listed as blocked, and can be let back in.
  await page.getByRole("tab", { name: "ブロック中" }).click();
  await expect(page.getByText(`@${troll.handle}`)).toBeVisible();
  await page.getByRole("button", { name: "解除する" }).click();
  await expect(page.getByText("trollさんのブロックを解除しました")).toBeVisible();
  await expect(page.getByText("ブロックしている人はいません")).toBeVisible();

  // Removing the word.
  await page.getByRole("tab", { name: "ルール" }).click();
  await page.getByRole("button", { name: "「ネタバレ」を外す" }).click();
  await expect(page.getByText("まだ登録していません")).toBeVisible();
});

test("mentions are suggested while typing and become links; a comment can be edited", async ({
  page,
  me: _me,
  request,
}) => {
  const { video } = await makeCreator(request);
  const friend = await createAccount(request, "mention");
  await openThread(page, video.shortId);

  const field = thread(page).getByLabel("コメントを書く…");
  await field.click();
  await field.pressSequentially(`すごい！ @${friend.handle.slice(0, 9)}`);
  const option = page.getByRole("option").filter({ hasText: `@${friend.handle}` });
  await expect(option).toBeVisible();
  await option.click();
  await expect(field).toHaveValue(`すごい！ @${friend.handle} `);
  await field.pressSequentially("も見て");
  await thread(page).getByRole("button", { name: "コメント", exact: true }).click();

  const comment = commentWith(page, "すごい！");
  await expect(comment.getByRole("link", { name: `@${friend.handle}` })).toHaveAttribute(
    "href",
    `/u/${friend.handle}`,
  );

  // Edit it: the text changes and says so. (While editing, the old words are not on screen to find the comment by.)
  await comment.getByRole("button", { name: "その他" }).click();
  await page.getByRole("menuitem", { name: "編集する" }).click();
  const editing = thread(page).getByTestId("comment").first();
  await editing.getByLabel("編集する").fill("やっぱりこっち！");
  await editing.getByRole("button", { name: "保存する" }).click();
  await expect(commentWith(page, "やっぱりこっち！")).toContainText("編集済み");
  await expect(commentWith(page, "すごい！")).toHaveCount(0);
});

test("a comment can carry a picture and a moment of the video", async ({
  page,
  me: _me,
  request,
}) => {
  const { video } = await makeCreator(request);
  await openThread(page, video.shortId);
  // Stop the video somewhere after the start, so "now" is a known moment.
  await expect
    .poll(
      () => page.locator("video").evaluate((element: HTMLVideoElement) => element.currentTime),
      { timeout: 30_000 },
    )
    .toBeGreaterThan(2);
  await page.keyboard.press("5");
  await page.keyboard.press("k");
  await expect(page.getByTestId("video-player")).toHaveAttribute("data-state", "paused");

  const field = thread(page).getByLabel("コメントを書く…");
  await field.fill("ここ好き");
  await thread(page)
    .getByRole("button", { name: /いまの再生位置（0:1\d）をつける/ })
    .click();
  await thread(page).locator('input[type="file"]').setInputFiles(picture());
  await expect(thread(page).getByRole("button", { name: "画像を外す" })).toBeVisible();
  // The post button waits for the upload.
  await expect(thread(page).getByRole("button", { name: "コメント", exact: true })).toBeEnabled({
    timeout: 30_000,
  });
  await thread(page).getByRole("button", { name: "コメント", exact: true }).click();

  const comment = commentWith(page, "ここ好き");
  await expect(comment.getByRole("button", { name: "画像を大きく見る" })).toBeVisible();
  // The moment is a button that takes the player there.
  await page.keyboard.press("0");
  await expect
    .poll(() => page.locator("video").evaluate((element: HTMLVideoElement) => element.currentTime))
    .toBeLessThan(2);
  await comment.getByRole("button", { name: /0:1\d にジャンプ/ }).click();
  await expect
    .poll(() => page.locator("video").evaluate((element: HTMLVideoElement) => element.currentTime))
    .toBeGreaterThanOrEqual(10);

  // The picture opens larger.
  await comment.getByRole("button", { name: "画像を大きく見る" }).click();
  await expect(page.getByRole("dialog").getByRole("img")).toBeVisible();
});

test("my comments lead back to where they were written, highlighted; newest-first sorting", async ({
  page,
  me,
  request,
}) => {
  const { video } = await makeCreator(request);
  const other = await createAccount(request, "other");
  const first = `さいしょのコメント ${suffix()}`;
  const second = `あとのコメント ${suffix()}`;
  await writeComment(request, other, video.id, first);
  const mine = await writeComment(request, me, video.id, second);

  // Newest first puts the later comment on top.
  await openThread(page, video.shortId);
  await thread(page).getByRole("tab", { name: "新しい順" }).click();
  await expect(thread(page).getByTestId("comment").first()).toContainText(second);
  await expect(thread(page).getByTestId("comment").nth(1)).toContainText(first);

  // "My comments" lists what this person wrote, with what it is on.
  await page.goto("/settings/comments?tab=mine");
  const row = page.getByTestId("comment-row").filter({ hasText: second });
  await expect(row).toContainText(video.title);
  await expect(page.getByTestId("comment-row").filter({ hasText: first })).toHaveCount(0);

  // The row opens the video at that comment.
  await row.getByRole("link").click();
  await expect(page).toHaveURL(new RegExp(`/watch/${video.shortId}\\?comment=${mine.id}`));
  await expect(page.locator(`[data-comment-id="${mine.id}"]`)).toHaveClass(/ring-brand/);
});

test("with comments turned off, viewers are told and cannot write", async ({
  page,
  me: _me,
  request,
}) => {
  const { owner, video } = await makeCreator(request);
  await writeComment(request, owner, video.id, "オフにする前のコメント");
  await rpc(request, `${VIDEOS}/UpdateVideo`, { videoId: video.id, commentsEnabled: false }, owner);

  await openThread(page, video.shortId);
  await expect(thread(page).getByText("コメントはオフになっています")).toBeVisible();
  await expect(thread(page).getByLabel("コメントを書く…")).toHaveCount(0);
  await expect(thread(page).getByTestId("comment")).toHaveCount(0);
});
