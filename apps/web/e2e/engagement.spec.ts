import type { APIRequestContext, Locator, Page } from "@playwright/test";
import { blinkingGif, discPicture, uploadPicture } from "./pictures";
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

test.describe.configure({ timeout: 90_000 });

const NOTES = "tokuly.notes.v1.NoteService";
const REACTIONS = "tokuly.engagement.v1.ReactionService";
const BOOKMARKS = "tokuly.engagement.v1.BookmarkService";
const ENGAGEMENT = "tokuly.engagement.v1.EngagementService";
const REPORTS = "tokuly.engagement.v1.ReportService";

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
const subject = (noteId: string) => ({ type: "SUBJECT_TYPE_NOTE", id: noteId });

/** A friend of `me` with a public note to engage with. Notes are the first content that carries the shared controls. */
async function friendWithNote(
  request: APIRequestContext,
  me: Account,
  text: string,
): Promise<{ friend: Account; noteId: string }> {
  const friend = await createAccount(request, "friend");
  await befriend(request, me, friend);
  const rules = await rpc<{ version: number }>(request, `${NOTES}/GetNoteRules`, {}, friend);
  await rpc(request, `${NOTES}/AgreeToNoteRules`, { version: rules.version }, friend);
  const created = await rpc<{ note: { id: string } }>(
    request,
    `${NOTES}/CreateNote`,
    { text },
    friend,
  );
  return { friend, noteId: created.note.id };
}

const card = (page: Page, text: string): Locator =>
  page.locator('[data-testid="note"]', { hasText: text });

interface EngagementJson {
  reactions?: {
    emojiCounts?: {
      emoji: string;
      count: string;
      custom?: { name: string; available?: boolean };
    }[];
    viewerReaction?: { emoji?: string };
  };
  bookmarked?: boolean;
  shareCount?: string;
}

async function engagementOf(
  request: APIRequestContext,
  noteId: string,
  as: Account,
): Promise<EngagementJson> {
  return (
    await rpc<{ engagement: EngagementJson }>(
      request,
      `${ENGAGEMENT}/GetEngagement`,
      { subject: subject(noteId) },
      as,
    )
  ).engagement;
}

test("an emoji reaction is added, joined by others live, listed and taken back", async ({
  page,
  me,
  request,
}) => {
  const text = `e2e リアクションされるノート ${tag()}`;
  const { friend, noteId } = await friendWithNote(request, me, text);

  await page.goto(`/notes/${noteId}`);
  const note = card(page, text);
  await note.getByRole("button", { name: "リアクションする" }).click();
  await page
    .getByTestId("emoji-picker")
    .getByRole("button", { name: "🎉", exact: true })
    .first()
    .click();

  const chip = note.getByRole("button", { name: /^🎉/ });
  await expect(chip).toHaveAttribute("aria-pressed", "true");
  await expect(chip).toContainText("1");
  expect((await engagementOf(request, noteId, me)).reactions?.viewerReaction?.emoji).toBe("🎉");

  // Someone else reacts with the same emoji: the chip counts up without a reload.
  await expect(async () => {
    await rpc(request, `${REACTIONS}/RemoveReaction`, { subject: subject(noteId) }, friend);
    await rpc(
      request,
      `${REACTIONS}/SetReaction`,
      { subject: subject(noteId), reaction: { kind: "REACTION_KIND_EMOJI", emoji: "🎉" } },
      friend,
    );
    await expect(chip).toContainText("2", { timeout: 4_000 });
  }).toPass({ timeout: 30_000 });

  // Who reacted.
  await note.getByRole("button", { name: "リアクションした人" }).click();
  const dialog = page.getByRole("dialog");
  await expect(dialog.getByText(`@${friend.handle}`)).toBeVisible();
  await expect(dialog.getByText(`@${me.handle}`)).toBeVisible();
  await page.keyboard.press("Escape");

  // A tap on one's own reaction takes it back.
  await chip.click();
  await expect(chip).toHaveAttribute("aria-pressed", "false");
  await expect(chip).toContainText("1");
  await expect
    .poll(async () => (await engagementOf(request, noteId, me)).reactions?.viewerReaction)
    .toBeUndefined();
});

test("a note is saved, found under bookmarks, filed into a collection and unsaved", async ({
  page,
  me,
  request,
}) => {
  const text = `e2e 保存するノート ${tag()}`;
  const { noteId } = await friendWithNote(request, me, text);

  await page.goto(`/notes/${noteId}`);
  await card(page, text).getByRole("button", { name: "保存する" }).click();
  await expect(page.getByText("保存したよ!")).toBeVisible();
  await expect(card(page, text).getByRole("button", { name: /保存済み/ })).toHaveAttribute(
    "aria-pressed",
    "true",
  );

  // It is on the bookmarks page, drawn as the note it is.
  await page.goto("/bookmarks");
  const saved = page.locator('[data-testid="bookmark"]', { hasText: text });
  await expect(saved.getByTestId("note")).toBeVisible();

  // File it into a collection made on the spot.
  const name = `e2e あとで読む ${tag()}`;
  await saved.getByRole("button", { name: "コレクション", exact: true }).click();
  const chooser = page.getByRole("dialog");
  await chooser.getByRole("textbox", { name: "新しいコレクション" }).fill(name);
  await chooser.getByRole("button", { name: "つくる" }).click();
  await expect(chooser.getByRole("checkbox", { name: new RegExp(name) })).toHaveAttribute(
    "aria-checked",
    "true",
  );
  await chooser.getByRole("button", { name: "完了" }).click();

  // The collection lists it; the page address remembers the collection.
  const collections = page.getByRole("navigation", { name: "コレクション" });
  await collections.getByRole("button", { name: new RegExp(name) }).click();
  await expect(page).toHaveURL(/collection=/);
  await expect(page.getByRole("heading", { name })).toBeVisible();
  await expect(saved).toBeVisible();

  // Rename the collection.
  await page.getByRole("button", { name: "コレクションのメニュー" }).click();
  await page.getByRole("menuitem", { name: "名前を変える" }).click();
  await page.getByRole("dialog").getByLabel("コレクションの名前").fill(`${name} 2`);
  await page.getByRole("dialog").getByRole("button", { name: "変更する" }).click();
  await expect(page.getByRole("heading", { name: `${name} 2` })).toBeVisible();

  // Taking it out of the collection keeps it saved.
  await saved.getByRole("button", { name: "ここから外す" }).click();
  await expect(page.getByText("このコレクションはからっぽ")).toBeVisible();
  await collections.getByRole("button", { name: /^ぜんぶ/ }).click();
  await expect(saved).toBeVisible();

  // Delete the collection, then stop saving the note.
  await collections.getByRole("button", { name: new RegExp(`${name} 2`) }).click();
  await page.getByRole("button", { name: "コレクションのメニュー" }).click();
  await page.getByRole("menuitem", { name: "コレクションを削除" }).click();
  await page.getByRole("dialog").getByRole("button", { name: "コレクションを削除" }).click();
  await expect(collections.getByRole("button", { name: new RegExp(name) })).toHaveCount(0);
  await saved.getByRole("button", { name: "はずす", exact: true }).click();
  await expect(page.getByText("まだ何も保存してないよ")).toBeVisible();
  const collectionsLeft = (
    await rpc<{ collections: { isDefault?: boolean }[] }>(
      request,
      `${BOOKMARKS}/ListCollections`,
      {},
      me,
    )
  ).collections;
  expect(collectionsLeft).toHaveLength(1);
  expect((await engagementOf(request, noteId, me)).bookmarked ?? false).toBe(false);
});

test("what I reacted to is listed with my bookmarks", async ({ page, me, request }) => {
  const text = `e2e リアクション一覧のノート ${tag()}`;
  const { noteId } = await friendWithNote(request, me, text);
  await rpc(
    request,
    `${REACTIONS}/SetReaction`,
    { subject: subject(noteId), reaction: { kind: "REACTION_KIND_EMOJI", emoji: "🔥" } },
    me,
  );

  await page.goto("/bookmarks?tab=reactions");
  const row = page.locator('[data-testid="reacted"]', { hasText: text });
  await expect(row.getByRole("img", { name: "あなたのリアクション" })).toHaveText("🔥");
  await row.getByRole("button", { name: "取り消す" }).click();
  await expect(row).toHaveCount(0);
  await expect(page.getByText("まだリアクションしてないよ")).toBeVisible();
});

test("sharing copies the note's link and is counted", async ({ page, me, request, context }) => {
  await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  const text = `e2e シェアするノート ${tag()}`;
  const { noteId } = await friendWithNote(request, me, text);

  await page.goto(`/notes/${noteId}`);
  await card(page, text).getByRole("button", { name: "シェアする" }).click();
  // The share control is a menu: send in chat, copy the link, and the share sheet where there is one.
  await page.getByRole("menuitem", { name: "リンクをコピー" }).click();
  await expect(page.getByText("リンクをコピーしたよ!")).toBeVisible();
  expect(await page.evaluate(() => navigator.clipboard.readText())).toContain(`/notes/${noteId}`);
  await expect.poll(async () => (await engagementOf(request, noteId, me)).shareCount).toBe("1");
});

test("a note is reported, shows under my reports, and reaches the moderation queue", async ({
  page,
  me,
  request,
  browser,
  baseURL,
}) => {
  const text = `e2e 通報されるノート ${tag()}`;
  const { friend, noteId } = await friendWithNote(request, me, text);

  await page.goto(`/notes/${noteId}`);
  await card(page, text).getByRole("button", { name: "メニュー" }).click();
  await page.getByRole("menuitem", { name: "通報する" }).click();
  const dialog = page.getByRole("dialog");
  // A reason is required.
  await expect(dialog.getByRole("button", { name: "通報する" })).toBeDisabled();
  await dialog.getByRole("radio", { name: /スパム/ }).click();
  const details = `e2e くわしく ${tag()}`;
  await dialog.getByLabel("くわしく (なくてもOK)").fill(details);
  await dialog.getByRole("button", { name: "通報する" }).click();
  await expect(page.getByText("通報を受け付けたよ")).toBeVisible();

  // Reporting the same note again is refused with an explanation.
  await card(page, text).getByRole("button", { name: "メニュー" }).click();
  await page.getByRole("menuitem", { name: "通報する" }).click();
  await page
    .getByRole("dialog")
    .getByRole("radio", { name: /スパム/ })
    .click();
  await page.getByRole("dialog").getByRole("button", { name: "通報する" }).click();
  await expect(page.getByText("もう通報済みだよ")).toBeVisible();
  await page.keyboard.press("Escape");

  // "My reports" lists it with where it stands.
  await page.goto("/activity?tab=reports");
  const report = page.locator('[data-testid="report"]', { hasText: text });
  await expect(report).toContainText("確認待ち");
  await expect(report).toContainText("スパム");
  await expect(report).toContainText(details);
  const mine = await rpc<{ reports: { subject: { id: string } }[] }>(
    request,
    `${REPORTS}/ListMyReports`,
    {},
    me,
  );
  expect(mine.reports.map((entry) => entry.subject.id)).toContain(noteId);

  // The moderation team finds it in their queue.
  const staff = await browser.newContext({
    baseURL,
    locale: "ja-JP",
    viewport: page.viewportSize(),
  });
  const console = await staff.newPage();
  await dismissDevServerOverlay(console);
  await signInAs(console, await adminAccount(request));
  await expect(async () => {
    await console.goto(`/admin/moderation?owner=${friend.userId}`);
    await expect(console.getByRole("link", { name: new RegExp(text) }).first()).toBeVisible({
      timeout: 8_000,
    });
  }).toPass({ timeout: 40_000 });
  await staff.close();
});

test("a profile can be reported from its page", async ({ page, me, request }) => {
  const other = await createAccount(request, "other");
  await page.goto(`/u/${other.handle}`);
  await page.getByRole("button", { name: "通報する" }).click();
  const dialog = page.getByRole("dialog");
  await expect(dialog).toContainText(`@${other.handle}`);
  await dialog.getByRole("radio", { name: /なりすまし/ }).click();
  await dialog.getByRole("button", { name: "通報する" }).click();
  await expect(page.getByText("通報を受け付けたよ")).toBeVisible();
  const mine = await rpc<{ reports: { subject: { type: string; id: string } }[] }>(
    request,
    `${REPORTS}/ListMyReports`,
    {},
    me,
  );
  expect(mine.reports[0]?.subject).toMatchObject({ type: "SUBJECT_TYPE_USER", id: other.userId });
});

test("a note is sent to a friend in chat from its share menu", async ({ page, me, request }) => {
  const text = `e2e チャットで送るノート ${tag()}`;
  const { friend, noteId } = await friendWithNote(request, me, text);

  await page.goto(`/notes/${noteId}`);
  await card(page, text).getByRole("button", { name: "シェアする" }).click();
  await page.getByRole("menuitem", { name: "チャットで送る" }).click();

  const sheet = page.getByRole("dialog", { name: "ともだちに送る" });
  await sheet.getByRole("button", { name: friend.displayName }).click();
  await sheet.getByLabel("ひとこと添える（なくてもOK）").fill("これ見て！");
  await sheet.getByRole("button", { name: "送る（1）" }).click();
  await expect(page.getByText("1件のトークに送りました！")).toBeVisible();
  await expect(sheet).toBeHidden();

  // The friend has the link in the conversation, and the note counts one share.
  const { conversation } = await rpc<{ conversation: { id: string } }>(
    request,
    "tokuly.chat.v1.ConversationService/OpenDirect",
    { userId: me.userId },
    friend,
  );
  const { messages } = await rpc<{ messages: { text?: string }[] }>(
    request,
    "tokuly.chat.v1.MessageService/ListMessages",
    { conversationId: conversation.id },
    friend,
  );
  expect(messages.map((message) => message.text ?? "").join("\n")).toContain(`/notes/${noteId}`);
  expect(messages.map((message) => message.text ?? "").join("\n")).toContain("これ見て！");
  await expect.poll(async () => (await engagementOf(request, noteId, me)).shareCount).toBe("1");

  // In the conversation the note shows as its preview.
  await page.goto(`/chat/${conversation.id}`);
  await expect(page.getByTestId("message").filter({ hasText: "これ見て！" })).toBeVisible();
});

// ───────────────────────────── Custom emoji reactions ─────────────────────────────

const EMOJI_ADMIN = "tokuly.emoji.v1.EmojiAdminService";

/** Emoji this file put into the catalogue (one for the whole server), taken out again after each test. */
const addedEmoji = new Set<string>();

test.afterEach(async ({ request }) => {
  if (addedEmoji.size === 0) return;
  const admin = await adminAccount(request);
  for (const id of addedEmoji)
    await rpc(request, `${EMOJI_ADMIN}/DeleteEmoji`, { emojiId: id }, admin).catch(() => {});
  addedEmoji.clear();
});

/** A custom emoji a moderator put straight into the catalogue: animated (a blinking GIF) or a still disc. */
async function customEmoji(
  request: APIRequestContext,
  options: { animated?: boolean; sensitive?: boolean } = {},
): Promise<{ id: string; name: string; code: string; url: string }> {
  const admin = await adminAccount(request);
  const name = `rx_${tag()}`;
  const file = options.animated ? blinkingGif(`${name}.gif`) : discPicture(`${name}.png`);
  const assetId = await uploadPicture(request, admin, file);
  const added = await rpc<{ emoji: { emoji: { id: string; image: { url: string } } } }>(
    request,
    `${EMOJI_ADMIN}/AddEmoji`,
    { name, assetId, sensitive: options.sensitive ?? false },
    admin,
  );
  addedEmoji.add(added.emoji.emoji.id);
  return { id: added.emoji.emoji.id, name, code: `:${name}:`, url: added.emoji.emoji.image.url };
}

test("a custom emoji is picked, drawn as its picture (still for less motion), and falls back once taken down", async ({
  page,
  me,
  request,
}) => {
  test.setTimeout(120_000);
  const emoji = await customEmoji(request, { animated: true });
  const text = `e2e カスタム絵文字でリアクション ${tag()}`;
  const { friend, noteId } = await friendWithNote(request, me, text);

  await page.goto(`/notes/${noteId}`);
  const note = card(page, text);
  await note.getByRole("button", { name: "リアクションする" }).click();
  const picker = page.getByTestId("emoji-picker");
  await picker.getByRole("searchbox").fill(emoji.name);
  await picker.getByRole("button", { name: emoji.code }).click();

  // The chip shows the emoji's picture, moving while motion is allowed.
  const chip = note.locator(`button[data-emoji="${emoji.code}"]`);
  await expect(chip).toHaveAttribute("aria-pressed", "true");
  await expect(chip).toContainText("1");
  const picture = chip.locator(`img[data-emoji="${emoji.name}"]`);
  await expect(picture).toHaveAttribute("src", emoji.url);
  // The server stored it and describes it for every client.
  await expect
    .poll(
      async () => (await engagementOf(request, noteId, friend)).reactions?.emojiCounts?.[0]?.emoji,
    )
    .toBe(emoji.code);

  // "Reduce motion": the still rendition instead.
  await page.emulateMedia({ reducedMotion: "reduce" });
  await expect(picture).not.toHaveAttribute("src", emoji.url);
  await expect(picture).toHaveAttribute("src", /.+/);
  await page.emulateMedia({ reducedMotion: "no-preference" });

  // Who reacted shows the picture too.
  await note.getByRole("button", { name: "リアクションした人" }).click();
  const dialog = page.getByRole("dialog");
  await expect(dialog.getByText(`@${me.handle}`)).toBeVisible();
  await expect(dialog.locator(`img[data-emoji="${emoji.name}"]`).first()).toBeVisible();
  await page.keyboard.press("Escape");

  // A moderator takes the emoji down: the reaction stays, shown as its shortcode, and can still be taken back.
  const admin = await adminAccount(request);
  await rpc(request, `${EMOJI_ADMIN}/DisableEmoji`, { emojiId: emoji.id, reason: "e2e" }, admin);
  await page.reload();
  const fallback = card(page, text).locator(`button[data-emoji="${emoji.code}"]`);
  await expect(fallback).toContainText(emoji.code);
  await expect(fallback.locator("img")).toHaveCount(0);
  await fallback.click();
  await expect(fallback).toHaveCount(0);
  await expect
    .poll(async () => (await engagementOf(request, noteId, me)).reactions?.viewerReaction)
    .toBeUndefined();
});

test("reactions others add pop up over the note while it is on screen", async ({
  page,
  me,
  request,
}) => {
  const emoji = await customEmoji(request);
  const text = `e2e ポップするリアクション ${tag()}`;
  const { friend, noteId } = await friendWithNote(request, me, text);
  await page.goto(`/notes/${noteId}`);
  const note = card(page, text);
  await expect(note.getByRole("button", { name: "リアクションする" })).toBeVisible();

  // The topic is followed a moment after the note settled on screen; keep reacting until a pop shows.
  await expect(async () => {
    await rpc(request, `${REACTIONS}/RemoveReaction`, { subject: subject(noteId) }, friend);
    await rpc(
      request,
      `${REACTIONS}/SetReaction`,
      { subject: subject(noteId), reaction: { kind: "REACTION_KIND_EMOJI", emoji: emoji.code } },
      friend,
    );
    await expect(note.getByTestId("reaction-pop").first()).toBeAttached({ timeout: 3_000 });
  }).toPass({ timeout: 30_000 });
  await expect(note.locator(`button[data-emoji="${emoji.code}"]`)).toContainText("1");
});

test("an owner decides about custom and sensitive emoji in the reaction settings", async ({
  page,
  me,
  request,
}) => {
  const spicy = await customEmoji(request, { sensitive: true });
  const fan = await createAccount(request, "fan");
  await befriend(request, me, fan);
  const rules = await rpc<{ version: number }>(request, `${NOTES}/GetNoteRules`, {}, me);
  await rpc(request, `${NOTES}/AgreeToNoteRules`, { version: rules.version }, me);
  const created = await rpc<{ note: { id: string } }>(
    request,
    `${NOTES}/CreateNote`,
    { text: `e2e センシティブ ${tag()}` },
    me,
  );
  const react = (code: string) =>
    rpc(
      request,
      `${REACTIONS}/SetReaction`,
      { subject: subject(created.note.id), reaction: { kind: "REACTION_KIND_EMOJI", emoji: code } },
      fan,
    );
  await expect(react(spicy.code)).rejects.toThrow(/SENSITIVE_EMOJI_NOT_ALLOWED/);

  await page.goto("/settings");
  await page.getByRole("link", { name: /あなたの投稿につけられる絵文字/ }).click();
  await expect(page).toHaveURL(/\/settings\/reactions$/);
  const sensitive = page.getByRole("switch", { name: /センシティブな絵文字も使えるようにする/ });
  await expect(sensitive).not.toBeChecked();
  await sensitive.click();
  await expect(sensitive).toBeChecked();
  await expect
    .poll(async () => {
      const { settings } = await rpc<{ settings: { sensitiveEmoji?: boolean } }>(
        request,
        `${REACTIONS}/GetReactionSettings`,
        {},
        me,
      );
      return settings.sensitiveEmoji ?? false;
    })
    .toBe(true);
  await react(spicy.code);

  // Custom emoji off: the sensitive switch goes with it, and only Unicode emoji are taken.
  await page.getByRole("switch", { name: /カスタム絵文字を使えるようにする/ }).click();
  await expect(sensitive).toBeDisabled();
  await expect
    .poll(() =>
      react(spicy.code).then(
        () => "ok",
        (error: Error) => error.message,
      ),
    )
    .toMatch(/CUSTOM_EMOJI_NOT_ALLOWED/);
  await react("🎉");
});

test("the picker leaves out sensitive emoji where the owner does not take them", async ({
  page,
  me,
  request,
}) => {
  const spicy = await customEmoji(request, { sensitive: true });
  const plain = await customEmoji(request);
  const text = `e2e センシティブなし ${tag()}`;
  const { noteId } = await friendWithNote(request, me, text);
  await page.goto(`/notes/${noteId}`);
  await card(page, text).getByRole("button", { name: "リアクションする" }).click();
  const picker = page.getByTestId("emoji-picker");
  await picker.getByRole("searchbox").fill(plain.name);
  await expect(picker.getByRole("button", { name: plain.code })).toBeVisible();
  await picker.getByRole("searchbox").fill(spicy.name);
  await expect(picker.getByRole("button", { name: spicy.code })).toHaveCount(0);
});
