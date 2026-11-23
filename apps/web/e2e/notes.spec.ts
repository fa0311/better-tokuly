import zlib from "node:zlib";
import type { APIRequestContext, Locator, Page } from "@playwright/test";
import { type Account, befriend, createAccount, expect, rpc, test } from "./support";

// Several tests post through the UI, upload a picture, or wait for a realtime push.
test.describe.configure({ timeout: 90_000 });

const NOTES = "tokuly.notes.v1.NoteService";
const ACTIVITY = "tokuly.feed.v1.ActivityService";

interface NoteJson {
  id: string;
  text: string;
}

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

/** Agrees to the posting rules as `account`, so its notes can be posted through the API. */
async function agree(request: APIRequestContext, account: Account): Promise<void> {
  const rules = await rpc<{ version: number }>(request, `${NOTES}/GetNoteRules`, {}, account);
  await rpc(request, `${NOTES}/AgreeToNoteRules`, { version: rules.version }, account);
}

async function post(
  request: APIRequestContext,
  account: Account,
  body: Record<string, unknown>,
): Promise<NoteJson> {
  return (await rpc<{ note: NoteJson }>(request, `${NOTES}/CreateNote`, body, account)).note;
}

/** A friend of `me` who has agreed to the rules. */
async function friendOf(
  request: APIRequestContext,
  me: Account,
  name = "friend",
): Promise<Account> {
  const friend = await createAccount(request, name);
  await befriend(request, me, friend);
  await agree(request, friend);
  return friend;
}

/** The card of the note whose text contains `text`. */
const card = (page: Page, text: string): Locator =>
  page.locator('[data-testid="note"]', { hasText: text });
/** The card of one note by ID (the note a page is about, or a row). */
const cardById = (page: Page, noteId: string): Locator =>
  page.locator(`[data-testid="note"][data-note-id="${noteId}"]`);
const action = (scope: Locator, name: string): Locator =>
  scope.getByRole("button", { name, exact: true });

/** A plain PNG picture. */
function picture(
  name: string,
  [red, green, blue]: [number, number, number],
  width = 320,
  height = 240,
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

const openComposer = (page: Page) =>
  page.getByRole("button", { name: "ノートを書く", exact: true }).last().click();
const composer = (page: Page) => page.getByTestId("composer");

test("the rules are agreed to before the first note, which then lands in the timeline", async ({
  page,
  me,
  request,
}) => {
  await page.goto("/notes?tab=home");
  await openComposer(page);

  // Nothing can be written before the rules were read and agreed to.
  const dialog = page.getByRole("dialog");
  await expect(dialog.getByRole("heading", { name: "ノートのルール" })).toBeVisible();
  await expect(page.getByTestId("note-rules").getByRole("listitem")).not.toHaveCount(0);
  await expect(composer(page)).toHaveCount(0);
  await dialog.getByRole("button", { name: "同意してはじめる" }).click();

  const text = `e2e はじめてのノート ${tag()}`;
  await composer(page).getByRole("textbox", { name: "今何してる?" }).fill(text);
  await expect(dialog.getByText(`${[...text].length}/300`)).toBeVisible();
  await dialog.getByRole("button", { name: "ノートする" }).click();

  await expect(page.getByText("ノートしたよ!", { exact: true })).toBeVisible();
  await expect(card(page, text)).toBeVisible();
  const rules = await rpc<{ agreed?: boolean }>(request, `${NOTES}/GetNoteRules`, {}, me);
  expect(rules.agreed).toBe(true);

  // The second time the composer opens straight away.
  await openComposer(page);
  await expect(composer(page)).toBeVisible();
});

test("a friend's notes arrive in an open home timeline", async ({ page, me, request }) => {
  const friend = await friendOf(request, me);
  const first = `e2e さいしょ ${tag()}`;
  await post(request, friend, { text: first });

  await page.goto("/notes?tab=home");
  await expect(card(page, first)).toBeVisible();

  // Live: a note the realtime connection announces (here through its mention) joins the list by itself.
  const live = `e2e ライブ ${tag()}`;
  await post(request, friend, { text: `${live} @${me.handle}` });
  await expect(card(page, live)).toBeVisible({ timeout: 30_000 });
  await expect(card(page, live).getByRole("link", { name: `@${me.handle}` })).toHaveAttribute(
    "href",
    `/u/${me.handle}`,
  );

  // A note nothing announced is there after a refresh.
  const quiet = `e2e しずかに ${tag()}`;
  await post(request, friend, { text: quiet });
  await page.getByRole("button", { name: "更新する" }).click();
  await expect(card(page, quiet)).toBeVisible();
  // Newest first.
  await expect(page.getByTestId("note").first()).toContainText(quiet);
});

test("new notes wait behind a pill while the reader is further down", async ({
  page,
  me,
  request,
}) => {
  const friend = await friendOf(request, me);
  for (let index = 0; index < 12; index++)
    await post(request, friend, { text: `e2e ならび ${index} ${tag()}\n\n\n.` });

  await page.goto("/notes?tab=home");
  await expect(card(page, "e2e ならび 11")).toBeVisible();
  await page.getByTestId("notes-scroller").evaluate((element) => element.scrollTo({ top: 1200 }));

  const fresh = `e2e あたらしい ${tag()}`;
  await post(request, friend, { text: `${fresh} @${me.handle}` });
  const pill = page.getByRole("button", { name: /新しいノート 1件/ });
  await expect(pill).toBeVisible({ timeout: 30_000 });
  await expect(card(page, fresh)).toHaveCount(0);

  await pill.click();
  await expect(card(page, fresh)).toBeVisible();
  await expect(pill).toHaveCount(0);
});

test("the home tab mixes notes with what else friends did", async ({ page, me, request }) => {
  const friend = await friendOf(request, me);
  const text = `e2e フィードのノート ${tag()}`;
  await post(request, friend, { text });

  await page.goto("/notes");
  await expect(page.getByRole("tab", { name: "ホーム" })).toHaveAttribute("aria-selected", "true");
  const feed = page.getByTestId("home-feed");
  await expect(feed.locator('[data-testid="note"]', { hasText: text })).toBeVisible();
  // Not a note: the two becoming friends, as a compact card that leads to the profile.
  await expect(
    feed.locator('[data-testid="feed-card"]', { hasText: "友達になったよ" }).first(),
  ).toBeVisible();

  // Everyone's public notes, and nothing but notes. (Not looked for by text: the global timeline
  // is shared with every test running at the same time, and a burst of their notes can push this
  // one past the first page.)
  await page.getByRole("tab", { name: "みんな" }).click();
  await expect(page).toHaveURL(/tab=global/);
  await expect(page.getByTestId("timeline").getByTestId("note").first()).toBeVisible();
  await expect(page.getByTestId("home-feed")).toHaveCount(0);
  await expect(page.getByTestId("feed-card")).toHaveCount(0);
});

test("likes, renotes, quotes and replies move the counters, live for everyone looking", async ({
  page,
  me,
  request,
}) => {
  await agree(request, me);
  const friend = await friendOf(request, me);
  const other = await friendOf(request, me, "other");
  const text = `e2e カウンター ${tag()}`;
  const note = await post(request, friend, { text });

  await page.goto(`/notes/${note.id}`);
  const focus = cardById(page, note.id);
  await expect(focus).toContainText(text);

  // Like: at once, with the count.
  const like = action(focus, "いいね");
  await like.click();
  await expect(like).toHaveAttribute("aria-pressed", "true");
  await expect(like).toHaveText("1");

  // Someone else likes it: the number moves without a reload.
  await expect(async () => {
    await rpc(request, `${NOTES}/UnlikeNote`, { noteId: note.id }, other);
    await rpc(request, `${NOTES}/LikeNote`, { noteId: note.id }, other);
    await expect(like).toHaveText("2", { timeout: 4_000 });
  }).toPass({ timeout: 30_000 });
  await page.getByRole("button", { name: /2\s*いいね/ }).click();
  await expect(page.getByTestId("likers").getByText(`@${other.handle}`)).toBeVisible();
  await page.keyboard.press("Escape");

  // Renote, and take it back.
  const renote = action(focus, "リノート");
  await renote.click();
  await page.getByRole("menuitem", { name: "リノートする" }).click();
  await expect(page.getByText("リノートしたよ!")).toBeVisible();
  await expect(renote).toHaveAttribute("aria-pressed", "true");
  await expect(renote).toHaveText("1");
  await renote.click();
  await page.getByRole("menuitem", { name: "リノートを取り消す" }).click();
  await expect(renote).toHaveAttribute("aria-pressed", "false");

  // Quote: the composer shows the note being quoted; the quote is then listed on the note's page.
  const quote = `e2e 引用するよ ${tag()}`;
  await renote.click();
  await page.getByRole("menuitem", { name: "引用する" }).click();
  await expect(composer(page)).toContainText(text);
  await composer(page).getByRole("textbox").fill(quote);
  await page.getByRole("dialog").getByRole("button", { name: "ノートする" }).click();
  await expect(page.getByText("ノートしたよ!", { exact: true })).toBeVisible();
  await page.getByRole("tab", { name: "引用 1" }).click();
  const quoted = page.getByTestId("quotes").locator('[data-testid="note"]', { hasText: quote });
  await expect(quoted.getByTestId("quote")).toContainText(text);

  // Reply from the box under the note.
  const reply = `e2e 返信だよ ${tag()}`;
  await page.getByTestId("reply-box").getByRole("textbox").fill(reply);
  await page.getByRole("button", { name: "返信を送る" }).click();
  await expect(page.getByText("返信したよ!")).toBeVisible();
  await page.getByRole("tab", { name: "返信 1" }).click();
  await expect(
    page.getByTestId("replies").locator('[data-testid="note"]', { hasText: reply }),
  ).toBeVisible();
  await expect(action(focus, "返信する")).toHaveText("1");

  const counters = (
    await rpc<{ note: { counters: Record<string, string> } }>(
      request,
      `${NOTES}/GetNote`,
      { noteId: note.id },
      me,
    )
  ).note.counters;
  expect(counters).toMatchObject({ likeCount: "2", quoteCount: "1", replyCount: "1" });
});

test("a note is deleted from its menu", async ({ page, me, request }) => {
  await agree(request, me);
  const text = `e2e けすノート ${tag()}`;
  const note = await post(request, me, { text });

  await page.goto("/notes?tab=home");
  await expect(card(page, text)).toBeVisible();
  await card(page, text).getByRole("button", { name: "メニュー" }).click();
  await page.getByRole("menuitem", { name: "削除する" }).click();
  await page.getByRole("dialog").getByRole("button", { name: "削除する" }).click();

  await expect(page.getByText("ノートを削除したよ")).toBeVisible();
  await expect(card(page, text)).toHaveCount(0);
  await expect(rpc(request, `${NOTES}/GetNote`, { noteId: note.id }, me)).rejects.toThrow(
    /NOTE_NOT_FOUND/,
  );

  // Its own page says that it is gone.
  await page.goto(`/notes/${note.id}`);
  await expect(page.getByText("このノートは見つからないよ")).toBeVisible();
});

test("a hashtag opens its page", async ({ page, me, request }) => {
  const friend = await friendOf(request, me);
  const hashtag = `e2eタグ${tag()}`;
  const text = `e2e ハッシュタグつき ${tag()}`;
  await post(request, friend, { text: `${text} #${hashtag}` });

  await page.goto("/notes?tab=home");
  await card(page, text)
    .getByRole("link", { name: `#${hashtag}` })
    .click();
  await expect(page).toHaveURL(new RegExp(`/tag/${encodeURIComponent(hashtag)}$`));
  await expect(page.getByRole("heading", { name: `#${hashtag}` })).toBeVisible();
  await expect(page.getByText("1件のノート")).toBeVisible();
  await expect(card(page, text)).toBeVisible();

  // The address the server uses for hashtags (search results) leads to the same page.
  await page.goto(`/notes/tags/${encodeURIComponent(hashtag)}`);
  await expect(page).toHaveURL(new RegExp(`/tag/${encodeURIComponent(hashtag)}$`));
  await expect(card(page, text)).toBeVisible();
});

test("a note carries a picture, which opens full screen", async ({ page, me, request }) => {
  await agree(request, me);
  await page.goto("/notes?tab=home");
  await openComposer(page);

  await page.getByTestId("note-files").setInputFiles(picture("sora.png", [79, 195, 255]));
  await expect(page.locator('[data-testid="upload"][data-status="ready"]')).toBeVisible({
    timeout: 60_000,
  });
  const text = `e2e 写真つき ${tag()}`;
  await composer(page).getByRole("textbox", { name: "今何してる?" }).fill(text);
  await page.getByRole("dialog").getByRole("button", { name: "ノートする" }).click();
  await expect(page.getByText("ノートしたよ!", { exact: true })).toBeVisible();

  const posted = card(page, text);
  const open = posted.getByRole("button", { name: /写真を開く/ });
  await expect(open.locator("img")).toBeVisible();
  await open.click();
  await expect(page.getByTestId("image-viewer").locator("img")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("image-viewer")).toHaveCount(0);
});

test("the composer completes hashtags and keeps an unfinished note as a draft", async ({
  page,
  me,
  request,
}) => {
  await agree(request, me);
  const friend = await friendOf(request, me);
  // A prefix of its own, so no other run's hashtag is offered first.
  const prefix = `e2e${tag()}`;
  const hashtag = `${prefix}ほかん`;
  await post(request, friend, { text: `#${hashtag}` });

  await page.goto("/notes?tab=home");
  // The keyboard shortcut works once the page is up.
  await expect(
    page.getByRole("button", { name: "ノートを書く", exact: true }).last(),
  ).toBeVisible();
  await page.keyboard.press("n");
  const field = composer(page).getByRole("textbox", { name: "今何してる?" });
  await field.pressSequentially(`たべた #${prefix}`);
  const option = page.getByRole("option", { name: new RegExp(`#${hashtag}`) });
  await expect(option).toBeVisible();
  await page.keyboard.press("Enter");
  await expect(field).toHaveValue(`たべた #${hashtag} `);

  // Closing keeps the text; it is back the next time, and can be thrown away.
  await page.getByRole("dialog").getByRole("button", { name: "close" }).click();
  await expect(page.getByText("下書きに保存したよ")).toBeVisible();
  await openComposer(page);
  await expect(composer(page).getByRole("textbox", { name: "今何してる?" })).toHaveValue(
    `たべた #${hashtag} `,
  );
  await page.getByRole("button", { name: "すてる" }).click();
  await expect(composer(page).getByRole("textbox", { name: "今何してる?" })).toHaveValue("");
});

test("a note is pinned to the profile and its audience changed", async ({ page, me, request }) => {
  await agree(request, me);
  const pinned = `e2e 固定するノート ${tag()}`;
  await post(request, me, { text: pinned });
  await post(request, me, { text: `e2e あとのノート ${tag()}` });

  await page.goto(`/u/${me.handle}`);
  await card(page, pinned).getByRole("button", { name: "メニュー" }).click();
  await page.getByRole("menuitem", { name: "プロフィールに固定" }).click();
  await expect(page.getByText("プロフィールに固定したよ!")).toBeVisible();
  // The pinned note leads the profile, marked as such.
  await expect(page.getByTestId("note").first()).toContainText(pinned);
  await expect(page.getByTestId("note").first()).toContainText("固定されたノート");

  await card(page, pinned).getByRole("button", { name: "メニュー" }).click();
  await page.getByRole("menuitem", { name: "公開範囲を変える" }).click();
  const dialog = page.getByRole("dialog");
  await dialog.getByRole("radio", { name: /ともだち/ }).click();
  await dialog.getByRole("button", { name: "変更する" }).click();
  await expect(page.getByText("公開範囲を変えたよ")).toBeVisible();
  await expect(card(page, pinned).getByRole("img", { name: "ともだち" })).toBeVisible();
});

test("mentions have a tab of their own", async ({ page, me, request }) => {
  const friend = await friendOf(request, me);
  const text = `e2e メンションだよ ${tag()}`;
  await post(request, friend, { text: `@${me.handle} ${text}` });
  await post(request, friend, { text: `e2e かんけいないノート ${tag()}` });

  await page.goto("/notes?tab=mentions");
  await expect(card(page, text)).toBeVisible();
  await expect(card(page, "e2e かんけいないノート")).toHaveCount(0);
});

test("the log lists what I did; an entry can be deleted and a kind hidden from others", async ({
  page,
  me,
  request,
}) => {
  await agree(request, me);
  await post(request, me, { text: `e2e ログにのるノート ${tag()}` });

  // The log is written from events, a moment after the fact.
  await expect(async () => {
    const log = await rpc<{ activities?: { kind: string }[] }>(
      request,
      `${ACTIVITY}/ListMyActivity`,
      {},
      me,
    );
    expect(log.activities?.some((activity) => activity.kind === "note.posted")).toBe(true);
  }).toPass({ timeout: 20_000 });

  await page.goto("/activity");
  const entry = page.locator('[data-testid="activity"]', { hasText: "ノートを投稿したよ" });
  await expect(entry).toBeVisible();

  // Hide notes from other people: the entry is marked.
  await page.getByRole("button", { name: "ログのメニュー" }).click();
  await page.getByRole("menuitem", { name: "見せる種類をえらぶ" }).click();
  const preferences = page.getByRole("dialog");
  await preferences.getByRole("switch").first().waitFor();
  await preferences.locator("label", { hasText: "note.posted" }).getByRole("switch").click();
  await expect(async () => {
    const kinds = (
      await rpc<{ kinds: { kind: string; visibleToOthers?: boolean }[] }>(
        request,
        `${ACTIVITY}/GetActivityPreferences`,
        {},
        me,
      )
    ).kinds;
    expect(kinds.find((kind) => kind.kind === "note.posted")?.visibleToOthers ?? false).toBe(false);
  }).toPass({ timeout: 10_000 });
  await page.keyboard.press("Escape");
  await expect(entry.getByTitle("非公開にした種類")).toBeVisible();

  // Delete the entry.
  await entry.getByRole("button", { name: "このログを消す" }).click();
  await page.getByRole("dialog").getByRole("button", { name: "消す", exact: true }).click();
  await expect(page.getByText("ログを消したよ")).toBeVisible();
  await expect(entry).toHaveCount(0);
});

test("the recap turns the month into cards", async ({ page, me, request }) => {
  await agree(request, me);
  await post(request, me, { text: `e2e まとめにのるノート ${tag()}` });
  await expect(async () => {
    const log = await rpc<{ activities?: unknown[] }>(
      request,
      `${ACTIVITY}/ListMyActivity`,
      {},
      me,
    );
    expect(log.activities?.length ?? 0).toBeGreaterThan(1);
  }).toPass({ timeout: 20_000 });

  await page.goto("/recap");
  const recap = page.getByTestId("recap");
  await expect(recap).toBeVisible();
  await expect(recap.getByText("件のログ", { exact: true })).toBeVisible();
  // The last card offers the recap to share.
  await recap
    .getByRole("button", { name: /^カード \d+\/\d+$/ })
    .last()
    .click();
  await expect(recap.getByRole("button", { name: "画像で保存" })).toBeVisible();

  // A year instead of a month.
  await page.getByRole("tab", { name: "年" }).click();
  await expect(page).toHaveURL(/p=\d{4}$/);
  await expect(page.getByTestId("recap")).toBeVisible();
});
