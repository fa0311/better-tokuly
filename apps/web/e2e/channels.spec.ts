import type { APIRequestContext, Locator, Page } from "@playwright/test";
import { type Account, createAccount, expect, rpc, test } from "./support";

// The tests post through the composer and wait on several round trips.
test.describe.configure({ timeout: 90_000 });

const NOTES = "tokuly.notes.v1.NoteService";
const CHANNELS = "tokuly.notes.v1.NoteChannelService";
const CLIPS = "tokuly.notes.v1.ClipService";

interface NoteJson {
  id: string;
  text: string;
}

interface ChannelJson {
  id: string;
  name: string;
}

const tag = () => `${Date.now().toString(36).slice(-5)}${Math.random().toString(36).slice(2, 6)}`;

test.beforeEach(async ({ page }) => {
  // The dev server's own hiccups show as an overlay that swallows clicks.
  await page.addLocatorHandler(page.locator("vite-error-overlay"), async (overlay) => {
    await overlay.evaluate((element) => element.remove());
  });
});

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

async function createChannel(
  request: APIRequestContext,
  owner: Account,
  name: string,
): Promise<ChannelJson> {
  return (
    await rpc<{ channel: ChannelJson }>(request, `${CHANNELS}/CreateChannel`, { name }, owner)
  ).channel;
}

const card = (scope: Page | Locator, text: string): Locator =>
  scope.locator('[data-testid="note"]', { hasText: text });
/** A note of the channel page with its moderator toolbar. */
const channelNote = (page: Page, text: string): Locator =>
  page
    .getByTestId("channel-timeline")
    .locator("div", { has: card(page, text) })
    .first();

test("a channel is created, posted into from the composer and followed", async ({
  page,
  me,
  request,
}) => {
  await agree(request, me);
  const name = `e2e 放課後部 ${tag()}`;
  await page.goto("/notes/channels");
  await page.getByRole("button", { name: "チャンネルをつくる" }).first().click();
  const dialog = page.getByRole("dialog");
  await dialog.getByRole("textbox", { name: "名前" }).fill(name);
  await dialog.getByRole("textbox", { name: "説明" }).fill("ゲームの話をしよう!");
  await dialog.getByRole("button", { name: "色 #4d9dff" }).click();
  await dialog.getByRole("button", { name: "つくる", exact: true }).click();
  await expect(page.getByText("チャンネルができたよ!")).toBeVisible();
  await expect(page.getByRole("heading", { name, level: 2 })).toBeVisible();
  await expect(page.getByText("ゲームの話をしよう!")).toBeVisible();
  await expect(page.getByText("まだノートがないよ")).toBeVisible();

  // Posting from the channel page puts the note into the channel.
  const text = `e2e チャンネルのノート ${tag()}`;
  await page.getByRole("button", { name: "このチャンネルに投稿" }).first().click();
  await page.getByTestId("composer").getByRole("textbox", { name: "今何してる?" }).fill(text);
  await dialog.getByRole("button", { name: "ノートする" }).click();
  await expect(page.getByText("ノートしたよ!", { exact: true })).toBeVisible();
  const posted = card(page.getByTestId("channel-timeline"), text);
  await expect(posted).toBeVisible();
  await expect(posted.getByRole("link", { name: `チャンネル: ${name}` })).toBeVisible();

  // Following counts the follower.
  await page.getByRole("button", { name: "フォローする" }).click();
  await expect(page.getByRole("button", { name: "フォロー中" })).toBeVisible();
  await expect(page.getByText("1人がフォロー")).toBeVisible();

  // The chip on the note leads back to the channel; the directory lists it under "フォロー中".
  await page.goto("/notes/channels?tab=followed");
  await expect(page.getByRole("link", { name: new RegExp(name) })).toBeVisible();
});

test("moderators pin and take out notes, and the owner names moderators and archives", async ({
  page,
  me,
  request,
}) => {
  const member = await createAccount(request, "member");
  const helper = await createAccount(request, "helper");
  await agree(request, member);
  const channel = await createChannel(request, me, `e2e 管理 ${tag()}`);
  const keep = await post(request, member, { text: `e2e 残す ${tag()}`, channelId: channel.id });
  const off = await post(request, member, {
    text: `e2e 話題ちがい ${tag()}`,
    channelId: channel.id,
  });

  await page.goto(`/notes/channels/${channel.id}`);
  await expect(card(page, keep.text)).toBeVisible({ timeout: 20_000 });
  await channelNote(page, keep.text).getByRole("button", { name: "ピン留め", exact: true }).click();
  await expect(page.getByText("ピン留めしたよ!")).toBeVisible();
  const pinned = page.getByRole("region", { name: "ピン留め" });
  await expect(card(pinned, keep.text)).toBeVisible();

  await channelNote(page, off.text).getByRole("button", { name: "チャンネルから外す" }).click();
  await page.getByRole("dialog").getByRole("button", { name: "チャンネルから外す" }).click();
  await expect(page.getByText("チャンネルから外したよ")).toBeVisible();
  await expect(card(page.getByTestId("channel-timeline"), off.text)).toHaveCount(0);
  // It stays on its author's profile, outside the channel.
  const note = await rpc<{ note: { channel?: { id: string } } }>(
    request,
    `${NOTES}/GetNote`,
    { noteId: off.id },
    member,
  );
  expect(note.note.channel?.id ?? "").toBe("");

  // Settings: a moderator by handle, then archiving.
  await page.getByRole("button", { name: "設定" }).click();
  const dialog = page.getByRole("dialog");
  await dialog.getByRole("tab", { name: "モデレーター" }).click();
  await dialog.getByRole("textbox", { name: "ユーザー名で追加" }).fill(`@${helper.handle}`);
  await dialog.getByRole("button", { name: "追加" }).click();
  await expect(page.getByText("モデレーターに追加したよ!")).toBeVisible();
  await expect(dialog.getByText(`@${helper.handle}`)).toBeVisible();
  await dialog.getByRole("tab", { name: "アーカイブ" }).click();
  await dialog.getByRole("button", { name: "アーカイブする" }).click();
  await page.getByRole("dialog").last().getByRole("button", { name: "アーカイブする" }).click();
  await expect(page.getByText("アーカイブしたよ")).toBeVisible();
  await expect(page.getByText(/このチャンネルはアーカイブされているよ/)).toBeVisible();
  await expect(page.getByRole("button", { name: "このチャンネルに投稿" })).toHaveCount(0);
});

test("the directory searches channel names and a stranger reads without managing", async ({
  page,
  me,
  request,
}) => {
  const owner = await createAccount(request, "owner");
  const word = tag();
  const match = await createChannel(request, owner, `e2e ${word} 音楽`);
  await createChannel(request, owner, `e2e ${tag()} 映画`);
  await page.goto("/notes/channels");
  await page.getByRole("searchbox", { name: "チャンネルをさがす" }).fill(word);
  await expect(page.getByRole("link", { name: new RegExp(match.name) })).toBeVisible();
  await expect(page.getByRole("link", { name: /映画/ })).toHaveCount(0);
  await page.getByRole("link", { name: new RegExp(match.name) }).click();
  await expect(page.getByRole("heading", { name: match.name, level: 2 })).toBeVisible();
  await expect(page.getByRole("button", { name: "設定" })).toHaveCount(0);
  expect(me.handle).toBeTruthy();
});

test("a note goes into a clip from its menu and the clip lists it", async ({
  page,
  me,
  request,
}) => {
  const author = await createAccount(request, "author");
  await agree(request, author);
  const note = await post(request, author, { text: `e2e クリップしたいノート ${tag()}` });
  const name = `e2e あとで読む ${tag()}`;

  await page.goto(`/notes/${note.id}`);
  const focused = card(page, note.text).first();
  await focused.getByRole("button", { name: "メニュー" }).click();
  await page.getByRole("menuitem", { name: "クリップに追加" }).click();
  const sheet = page.getByRole("dialog");
  await sheet.getByRole("button", { name: "新しいクリップ" }).click();
  await sheet.getByRole("textbox", { name: "名前" }).fill(name);
  await sheet.getByRole("switch").click();
  await sheet.getByRole("button", { name: "つくる", exact: true }).click();
  await expect(page.getByText(`「${name}」に追加したよ!`)).toBeVisible();
  await expect(sheet.getByRole("button", { name: new RegExp(name), pressed: true })).toBeVisible();
  await page.keyboard.press("Escape");

  // The public clip counts towards the note's clip count.
  const counted = await rpc<{ note: { counters?: { clipCount?: string } } }>(
    request,
    `${NOTES}/GetNote`,
    { noteId: note.id },
    author,
  );
  expect(Number(counted.note.counters?.clipCount ?? 0)).toBe(1);

  await page.goto("/notes/clips");
  const row = page.getByRole("link", { name: new RegExp(name) });
  await expect(row).toContainText("ノート 1");
  await expect(row).toContainText("公開");
  await row.click();
  await expect(card(page.getByTestId("clip-notes"), note.text)).toBeVisible();
  await page.getByRole("button", { name: "クリップから外す" }).click();
  await expect(page.getByText("クリップから外したよ")).toBeVisible();
  await expect(page.getByText("このクリップにはまだノートがないよ")).toBeVisible();

  // Others see the public clip, through the owner's clips.
  const clips = await rpc<{ clips: { name: string }[] }>(
    request,
    `${CLIPS}/ListClips`,
    { userId: me.userId },
    author,
  );
  expect(clips.clips.map((clip) => clip.name)).toContain(name);
});
