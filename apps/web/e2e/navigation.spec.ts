import type { Page } from "@playwright/test";
import { befriend, createAccount, expect, failToLoadPage, PASSWORD, rpc, test } from "./support";

/**
 * The frame every feature lives in: the tab bar and the rail, the hub and
 * the launcher, the quick search as a launcher, and the way back to a public
 * page after signing up.
 */

const mainNav = (page: Page) => page.getByRole("navigation", { name: "main" });
/** The item of the tab bar or the rail that is lit. */
const current = (page: Page) => mainNav(page).locator('[aria-current="page"]:visible');
const isPhone = (page: Page) => (page.viewportSize()?.width ?? 1280) < 768;

test("the hub groups every area into named sections, with search on top", async ({
  page,
  me: _me,
}) => {
  await page.goto("/me");
  const main = page.getByRole("main");
  for (const section of ["あそぶ", "おでかけ", "つくる・みる", "アカウント"])
    await expect(main.getByRole("heading", { level: 2, name: section, exact: true })).toBeVisible();

  // A tile sits in its section, and leads to its area.
  const play = main.getByRole("region", { name: "あそぶ", exact: true });
  await expect(play.getByRole("link", { name: "ともだち" })).toHaveAttribute("href", "/friends");
  await expect(play.getByRole("link", { name: /スクール/ })).toHaveAttribute("href", "/school");
  const account = main.getByRole("region", { name: "アカウント", exact: true });
  await expect(account.getByRole("link", { name: "ウォレット" })).toHaveAttribute(
    "href",
    "/wallet",
  );
  await expect(account.getByRole("link", { name: "管理" })).toHaveCount(0);

  // The search field of the hub opens the quick search: this is the way in on phones.
  await main.getByRole("button", { name: "ひと、タグ、機能をさがす" }).click();
  const palette = page.getByRole("dialog", { name: "クイック検索" });
  await expect(palette.getByRole("combobox")).toBeFocused();

  // The quick search knows the app's own places by name and by keyword, and goes there.
  await palette.getByRole("combobox").fill("ウォレ");
  await palette.getByRole("option", { name: "ウォレット" }).click();
  await expect(page).toHaveURL(/\/wallet$/);
});

test("the primary item a page belongs to stays lit on nested pages", async ({ page, me: _me }) => {
  const cases: [path: string, item: string][] = [
    ["/map", "マップ"],
    ["/places", "マップ"],
    ["/chat", "チャット"],
    ["/notes", "ノート"],
    ["/tag/e2e", "ノート"],
    ["/bookmarks", "ノート"],
    ["/activity", "ノート"],
    ["/stories/archive", "ノート"],
    ["/watch", "みる"],
    ["/watch/playlists", "みる"],
    ["/studio", "みる"],
  ];
  for (const [path, item] of cases) {
    await page.goto(path);
    await expect(current(page), path).toHaveCount(1);
    await expect(current(page), path).toHaveAccessibleName(item);
  }

  // Everything that is opened from the hub lights the hub's own slot.
  for (const path of ["/me", "/wallet", "/settings/appearance", "/friends"]) {
    await page.goto(path);
    await expect(current(page), path).toHaveAccessibleName("じぶん");
  }
});

test("on a phone a conversation takes the tab bar's place, and gives it back", async ({
  page,
  me,
  request,
}) => {
  test.skip(!isPhone(page), "the rail of wide screens never leaves");
  const friend = await createAccount(request, "friend");
  await befriend(request, me, friend);
  const { conversation } = await rpc<{ conversation: { id: string } }>(
    request,
    "tokuly.chat.v1.ConversationService/OpenDirect",
    { userId: friend.userId },
    me,
  );

  await page.goto("/chat");
  await expect(mainNav(page)).toBeVisible();
  await page.goto(`/chat/${conversation.id}`);
  await expect(page.getByRole("textbox", { name: "メッセージを入力" })).toBeVisible();
  await expect(mainNav(page)).toBeHidden();

  // The composer reaches the bottom edge: nothing is left where the bar was.
  const composer = await page.getByTestId("composer").boundingBox();
  expect(composer && composer.y + composer.height).toBeGreaterThan(
    (page.viewportSize()?.height ?? 0) - 8,
  );

  await page.goBack();
  await expect(mainNav(page)).toBeVisible();
});

test("on a wide screen the rail opens search and a launcher over the page that is showing", async ({
  page,
  me: _me,
}) => {
  test.skip(isPhone(page), "phones use the hub page");
  await page.goto("/notes");
  const rail = mainNav(page);

  await rail.getByRole("button", { name: "さがす" }).click();
  const palette = page.getByRole("dialog", { name: "クイック検索" });
  await expect(palette).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(palette).toBeHidden();

  await rail.getByRole("button", { name: "アプリ" }).click();
  const launcher = page.getByRole("dialog", { name: "アプリ" });
  for (const section of ["あそぶ", "おでかけ", "つくる・みる", "アカウント"])
    await expect(launcher.getByRole("heading", { name: section, exact: true })).toBeVisible();
  // Still on the page it was opened from.
  await expect(page).toHaveURL(/\/notes$/);

  await launcher.getByRole("link", { name: "あしあと" }).click();
  await expect(page).toHaveURL(/\/footprints$/);
  await expect(launcher).toBeHidden();
  await expect(current(page)).toHaveAccessibleName("マップ");
});

test("a toast on a phone leaves the page header and its controls free", async ({
  page,
  context,
  me: _me,
}) => {
  test.skip(!isPhone(page), "wide screens have room beside the header");
  await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  await page.goto("/friends");
  await page.getByRole("tab", { name: "招待" }).click();
  await page.getByRole("button", { name: "リンクをコピー" }).click();
  const shown = page.locator("[data-sonner-toast]", { hasText: "コピーしました!" });
  await expect(shown).toBeVisible();

  // Once it has slid in, it lies below the header (back, title, menu) and above the tab bar.
  const header = await page.locator("header").first().boundingBox();
  const bar = await mainNav(page).locator(":scope > div").boundingBox();
  await expect(async () => {
    const box = await shown.boundingBox();
    expect(box?.y).toBeGreaterThan((header?.y ?? 0) + (header?.height ?? 0));
    expect((box?.y ?? 0) + (box?.height ?? 0)).toBeLessThanOrEqual(bar?.y ?? 0);
  }).toPass({ timeout: 5_000 });
});

test("the settings list leads to comment moderation", async ({ page, me: _me }) => {
  await page.goto("/settings");
  await page
    .getByRole("navigation", { name: "設定" })
    .getByRole("link", { name: /^コメントの管理/ })
    .click();
  await expect(page).toHaveURL(/\/settings\/comments/);
  await expect(page.getByRole("heading", { level: 1, name: "コメントの管理" })).toBeVisible();
});

test("the recap a notification points at opens that month", async ({ page, me: _me }) => {
  await page.goto("/recap/2026-10");
  await expect(page).toHaveURL(/\/recap\?p=2026-10$/);
  // Something that is no month or year opens the recap of this month.
  await page.goto("/recap/soon");
  await expect(page).toHaveURL(/\/recap$/);
});

test("someone who signs up from a public page comes back to it", async ({ page, request }) => {
  const owner = await createAccount(request, "boxowner");
  const handle = `back_${Date.now().toString(36).slice(-6)}${Math.random().toString(36).slice(2, 5)}`;

  // A visitor of a question box goes to sign in, has no account, and makes one.
  await page.goto(`/q/${owner.handle}`);
  await page.getByRole("link", { name: "ログイン" }).first().click();
  await expect(page).toHaveURL(/\/login\?redirect=/);
  await page.getByRole("link", { name: "新しくはじめる" }).click();
  await expect(page).toHaveURL(/\/signup\?redirect=/);

  await page.getByLabel("なまえ").fill("もどる人");
  await page.getByLabel("ハンドル").fill(handle);
  await page.getByLabel("メールアドレス").fill(`${handle}@e2e.tokuly.test`);
  await page.getByLabel("パスワード").fill(PASSWORD);
  await page.getByLabel("誕生日").fill("2000-04-02");
  await page.getByRole("checkbox").check();
  await page.locator('form button[type="submit"]').click();

  // Back at the box they came from, inside the app now.
  await expect(page).toHaveURL(new RegExp(`/ask/u/${owner.handle}$`));
  await expect(mainNav(page)).toBeVisible();
});

test("a redirect that would leave the app is ignored", async ({ page }) => {
  await page.goto("/signup?redirect=//example.com/");
  await expect(page).toHaveURL(/\/signup$/);
  await page.goto("/login?redirect=https://example.com/");
  await expect(page).toHaveURL(/\/login$/);
});

test("a page whose code cannot load says so inside the app, with the navigation, and loads when tried again", async ({
  page,
  me: _me,
}) => {
  await page.goto("/me");
  await expect(page.getByRole("heading", { name: "じぶん" })).toBeVisible();
  // The page's code fails to arrive, as when the connection drops right as it is opened.
  const letThrough = await failToLoadPage(page, "announcements.index.tsx");

  await page.getByRole("main").getByRole("link", { name: "アップデート情報" }).click();
  await expect(page.getByRole("heading", { name: "うまくいきませんでした" })).toBeVisible();
  await expect(page.getByRole("navigation", { name: "main" })).toBeVisible();

  await letThrough();
  await page.getByRole("button", { name: "もう一度ためす" }).click();
  await expect(page.getByRole("heading", { level: 1, name: "アップデート情報" })).toBeVisible();
  await expect(page.getByRole("heading", { name: "うまくいきませんでした" })).toHaveCount(0);
});

test("a page the server rendered is taken over as it is, its words loaded first", async ({
  page,
}) => {
  // React throws the server's page away (and says so) when the first render in the browser
  // differs from it: a screen that showed nothing while its strings were still loading did.
  const thrownAway: string[] = [];
  page.on("pageerror", (error) => thrownAway.push(`${page.url()}: ${error.message}`));
  for (const path of ["/login", "/signup"]) {
    await page.goto(path);
    await expect(page.getByRole("heading", { level: 1 })).toBeVisible();
    // The browser has taken the page over once its code and strings have arrived.
    await page.waitForLoadState("networkidle");
  }
  expect(thrownAway).toEqual([]);
});
