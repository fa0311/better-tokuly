import type { APIRequestContext, Page } from "@playwright/test";
import { type Account, createAccount, expect, rpc, test } from "./support";

// Antennas are filled by a background job: a test waits for it.
test.describe.configure({ timeout: 90_000 });

const NOTES = "tokuly.notes.v1.NoteService";
const LISTS = "tokuly.notes.v1.NoteListService";

const word = () => `w${Date.now().toString(36).slice(-5)}${Math.random().toString(36).slice(2, 6)}`;

test.beforeEach(async ({ page }) => {
  await page.addLocatorHandler(page.locator("vite-error-overlay"), async (overlay) => {
    await overlay.evaluate((element) => element.remove());
  });
});

/** A new account that has agreed to the posting rules. */
async function poster(request: APIRequestContext, name: string): Promise<Account> {
  const account = await createAccount(request, name);
  const rules = await rpc<{ version: number }>(request, `${NOTES}/GetNoteRules`, {}, account);
  await rpc(request, `${NOTES}/AgreeToNoteRules`, { version: rules.version }, account);
  return account;
}

async function post(request: APIRequestContext, account: Account, text: string): Promise<void> {
  await rpc(request, `${NOTES}/CreateNote`, { text }, account);
}

const note = (page: Page, text: string) => page.locator('[data-testid="note"]', { hasText: text });

test("a list is made, filled from a profile and read as a timeline", async ({
  page,
  request,
  me,
}) => {
  const author = await poster(request, "author");
  const text = `list note ${word()}`;
  await post(request, author, text);

  await page.goto("/notes/lists");
  await expect(page.getByText("まだリストがないよ")).toBeVisible();
  await page.getByRole("button", { name: "リストを作ろう!" }).click();
  const dialog = page.getByRole("dialog");
  await dialog.getByLabel("名前").fill("推したち");
  await dialog.getByRole("switch").click();
  await dialog.getByRole("button", { name: "作成" }).click();
  await expect(page).toHaveURL(/\/notes\/lists\/[0-9a-f-]+$/);
  await expect(page.getByRole("heading", { name: "推したち" })).toBeVisible();
  await expect(page.getByTestId("list-counts")).toContainText("公開");

  // From the author's profile: リストに追加.
  await page.goto(`/u/${author.handle}`);
  await page.getByRole("button", { name: "リストに追加" }).click();
  await page.getByRole("dialog").getByRole("button", { name: "追加: 推したち" }).click();
  await expect(page.getByRole("dialog").getByText("追加したよ")).toBeVisible();
  await page.keyboard.press("Escape");

  await page.goto("/notes/lists");
  await page.getByTestId("note-list").filter({ hasText: "推したち" }).click();
  await expect(note(page, text)).toBeVisible();
  await expect(page.getByTestId("list-counts")).toContainText("1人");

  // Members: the author is there and can be taken off again.
  await page.getByRole("tab", { name: "メンバー" }).click();
  await expect(page.getByTestId("list-member")).toHaveCount(1);
  await page.getByRole("button", { name: `${author.displayName}さんをリストから外す` }).click();
  await expect(page.getByTestId("list-member")).toHaveCount(0);
  await expect(page.getByTestId("list-counts")).toContainText("0人");
  const lists = await rpc<{ lists: { memberCount?: number }[] }>(
    request,
    `${LISTS}/ListLists`,
    {},
    me,
  );
  expect(lists.lists[0]?.memberCount ?? 0).toBe(0);
});

test("someone else's public list can be followed, and left when one is on it", async ({
  page,
  request,
  me,
}) => {
  const owner = await poster(request, "owner");
  const created = await rpc<{ list: { id: string } }>(
    request,
    `${LISTS}/CreateList`,
    { name: "みんなのリスト", public: true },
    owner,
  );
  const listId = created.list.id;
  await rpc(request, `${LISTS}/AddListMembers`, { listId, userIds: [me.userId] }, owner);

  await page.goto("/notes/lists");
  await page.getByRole("tab", { name: "入ってるリスト" }).click();
  await page.getByTestId("note-list").filter({ hasText: "みんなのリスト" }).click();
  await expect(page.getByRole("heading", { name: "みんなのリスト" })).toBeVisible();

  await page.getByRole("button", { name: "フォロー", exact: true }).click();
  await expect(page.getByRole("button", { name: "フォロー中" })).toBeVisible();
  await expect(page.getByTestId("list-counts")).toContainText("フォロワー 1");

  await page.getByRole("button", { name: "編集" }).click();
  await page.getByRole("menuitem", { name: "リストから抜ける" }).click();
  await page.getByRole("dialog").getByRole("button", { name: "抜ける" }).click();
  await expect(page.getByText("リストから抜けたよ")).toBeVisible();
  const members = await rpc<{ members?: unknown[] }>(
    request,
    `${LISTS}/ListListMembers`,
    { listId },
    owner,
  );
  expect(members.members ?? []).toHaveLength(0);

  await page.goto("/notes/lists");
  await page.getByRole("tab", { name: "フォロー中" }).click();
  await expect(page.getByTestId("note-list").filter({ hasText: "みんなのリスト" })).toBeVisible();
});

test("an antenna catches new notes with its words", async ({ page, request, me }) => {
  expect(me.userId).toBeTruthy();
  const author = await poster(request, "author");
  const keyword = word();

  await page.goto("/notes/antennas");
  await expect(page.getByText("まだアンテナがないよ")).toBeVisible();
  await page.getByRole("button", { name: "アンテナを作ろう!" }).click();
  const dialog = page.getByRole("dialog");
  await dialog.getByLabel("名前").fill("推し活");
  await dialog.getByLabel("グループ1").fill(keyword.toUpperCase());
  await dialog.getByLabel("除外するワード").fill("ネタバレ");
  await dialog.getByRole("button", { name: "作成" }).click();
  await expect(page).toHaveURL(/\/notes\/antennas\/[0-9a-f-]+$/);
  await expect(page.getByTestId("antenna-words")).toHaveText(keyword);

  const caught = `${keyword} のライブ最高!`;
  await post(request, author, caught);
  await post(request, author, `${keyword} ネタバレ注意`);
  await post(request, author, "関係ないノート");
  // The match runs in the background; the page's refresh button reads again.
  await expect(async () => {
    await page.getByRole("button", { name: "更新する" }).click();
    await expect(note(page, caught)).toBeVisible({ timeout: 2_000 });
  }).toPass({ timeout: 30_000 });
  await expect(page.locator('[data-testid="note"]')).toHaveCount(1);

  await page.goto("/notes/antennas");
  await expect(page.getByTestId("antenna").filter({ hasText: "推し活" })).toBeVisible();
});
