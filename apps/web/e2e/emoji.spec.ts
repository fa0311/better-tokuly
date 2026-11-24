import zlib from "node:zlib";
import type { APIRequestContext, Browser, Page, TestInfo } from "@playwright/test";
import { blinkingGif, discPicture, type Rgb, type TestFile, uploadPicture } from "./pictures";
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

// These tests upload pictures and wait for the server to process them.
test.describe.configure({ timeout: 120_000 });

const EMOJI = "tokuly.emoji.v1.EmojiService";
const SUBMISSIONS = "tokuly.emoji.v1.EmojiSubmissionService";
const ADMIN = "tokuly.emoji.v1.EmojiAdminService";

// ───────────────────────────── Files made for the tests ─────────────────────────────

/** A ZIP archive of the given files (deflated). */
function zipArchive(files: Record<string, Buffer | string>): Buffer {
  const locals: Buffer[] = [];
  const directory: Buffer[] = [];
  let offset = 0;
  for (const [name, content] of Object.entries(files)) {
    const data = typeof content === "string" ? Buffer.from(content) : content;
    const packed = zlib.deflateRawSync(data);
    const fileName = Buffer.from(name);
    const checksum = zlib.crc32(data) >>> 0;
    const local = Buffer.alloc(30);
    local.writeUInt32LE(0x04034b50, 0);
    local.writeUInt16LE(20, 4);
    local.writeUInt16LE(8, 8);
    local.writeUInt32LE(checksum, 14);
    local.writeUInt32LE(packed.length, 18);
    local.writeUInt32LE(data.length, 22);
    local.writeUInt16LE(fileName.length, 26);
    const central = Buffer.alloc(46);
    central.writeUInt32LE(0x02014b50, 0);
    central.writeUInt16LE(20, 4);
    central.writeUInt16LE(20, 6);
    central.writeUInt16LE(8, 10);
    central.writeUInt32LE(checksum, 16);
    central.writeUInt32LE(packed.length, 20);
    central.writeUInt32LE(data.length, 24);
    central.writeUInt16LE(fileName.length, 28);
    central.writeUInt32LE(offset, 42);
    locals.push(local, fileName, packed);
    directory.push(central, fileName);
    offset += 30 + fileName.length + packed.length;
  }
  const directorySize = directory.reduce((sum, part) => sum + part.length, 0);
  const end = Buffer.alloc(22);
  end.writeUInt32LE(0x06054b50, 0);
  end.writeUInt16LE(Object.keys(files).length, 8);
  end.writeUInt16LE(Object.keys(files).length, 10);
  end.writeUInt32LE(directorySize, 12);
  end.writeUInt32LE(offset, 16);
  return Buffer.concat([...locals, ...directory, end]);
}

/** An emoji pack the way Misskey exports one: `meta.json` next to the pictures it names. */
function misskeyPack(
  entries: { name: string; file: TestFile; category?: string; aliases?: string[] }[],
): TestFile {
  const meta = {
    metaVersion: 2,
    host: "misskey.e2e.example",
    exportedAt: new Date().toISOString(),
    emojis: entries.map((entry, index) => ({
      fileName: entry.file.name,
      downloaded: true,
      emoji: {
        id: `e2e${index}`,
        updatedAt: new Date().toISOString(),
        name: entry.name,
        host: null,
        category: entry.category ?? null,
        originalUrl: `https://misskey.e2e.example/files/${entry.file.name}`,
        publicUrl: `https://misskey.e2e.example/files/${entry.file.name}`,
        uri: null,
        type: entry.file.mimeType,
        aliases: entry.aliases ?? [],
        license: "CC0",
        localOnly: false,
        isSensitive: false,
        roleIdsThatCanBeUsedThisEmojiAsReaction: [],
      },
    })),
  };
  const files: Record<string, Buffer | string> = { "meta.json": JSON.stringify(meta) };
  for (const entry of entries) files[entry.file.name] = entry.file.buffer;
  return { name: "emoji-pack.zip", mimeType: "application/zip", buffer: zipArchive(files) };
}

// ───────────────────────────── Arranging state through the API ─────────────────────────────

let counter = 0;
/** A name no other test (or run) uses: emoji names are unique for the whole server. */
const unique = (label: string) =>
  `${label}_${Date.now().toString(36).slice(-6)}${(counter++).toString(36)}${Math.random().toString(36).slice(2, 5)}`;

/**
 * The catalogue is one for the whole server, so what a test puts into it is
 * taken out again afterwards: emoji by name, categories by name.
 */
const leftovers = { emoji: new Set<string>(), categories: new Set<string>() };

test.afterEach(async ({ request }) => {
  if (leftovers.emoji.size === 0 && leftovers.categories.size === 0) return;
  const admin = await adminAccount(request);
  // Whatever is gone already (deleted or withdrawn by the test itself) is fine.
  for (const name of leftovers.emoji)
    await rpc(request, `${ADMIN}/DeleteEmoji`, { emojiId: name }, admin).catch(() => {});
  for (const name of leftovers.categories)
    await rpc(request, `${ADMIN}/DeleteCategory`, { name }, admin).catch(() => {});
  leftovers.emoji.clear();
  leftovers.categories.clear();
});

interface Arranged {
  id: string;
  name: string;
  code: string;
}

/** An emoji a moderator put straight into the catalogue. */
async function catalogEmoji(
  request: APIRequestContext,
  options: { label?: string; aliases?: string[]; color?: Rgb } = {},
): Promise<Arranged> {
  const admin = await adminAccount(request);
  const name = unique(options.label ?? "neko");
  leftovers.emoji.add(name);
  const assetId = await uploadPicture(request, admin, discPicture(`${name}.png`, options.color));
  const added = await rpc<{ emoji: { emoji: { id: string } } }>(
    request,
    `${ADMIN}/AddEmoji`,
    { name, assetId, aliases: options.aliases ?? [], license: "e2e" },
    admin,
  );
  return { id: added.emoji.emoji.id, name, code: `:${name}:` };
}

/** Another person's browser, signed in, at the size of the running project. */
async function browserOf(browser: Browser, testInfo: TestInfo, account: Account): Promise<Page> {
  const context = await browser.newContext({
    baseURL: testInfo.project.use.baseURL,
    locale: "ja-JP",
    timezoneId: "Asia/Tokyo",
    viewport: testInfo.project.use.viewport,
    hasTouch: testInfo.project.use.hasTouch,
  });
  const page = await context.newPage();
  await signInAs(page, account);
  return page;
}

// Tests that only need a signed-in browser ask for the `me` fixture without using the account (`me: _me`).

const deckOf = async (request: APIRequestContext, as: Account) =>
  (await rpc<{ items?: { emoji: string }[] }>(request, `${EMOJI}/GetDeck`, {}, as)).items?.map(
    (item) => item.emoji,
  ) ?? [];

// ───────────────────────────── The picker ─────────────────────────────

test("the picker finds an emoji by a Japanese alias and by name, and pins what is picked", async ({
  page,
  request,
  me,
}) => {
  const tag = unique("x").replace(/_/g, "");
  const emoji = await catalogEmoji(request, { aliases: [`ねこ${tag}`, "cat"] });

  await page.goto("/emoji/my-deck");
  await page.getByRole("button", { name: "追加" }).click();
  const picker = page.getByTestId("emoji-picker");
  await expect(picker).toBeVisible();
  // The deck comes first: a new account has the server's default one.
  await expect(
    picker.getByRole("region", { name: "マイデッキ" }).getByRole("button", { name: "👍" }),
  ).toBeVisible();

  // By alias, typed in katakana: kana and width make no difference.
  const search = picker.getByRole("searchbox");
  await search.fill(`ネコ${tag}`);
  await expect(picker.getByRole("button", { name: emoji.code })).toBeVisible();
  await expect(picker.locator("[data-emoji-cell]")).toHaveCount(1);

  // By name, with the colons people copy along.
  await search.fill(`:${emoji.name.slice(0, -2)}`);
  await expect(picker.getByRole("button", { name: emoji.code })).toBeVisible();

  // Unicode emoji are found in Japanese too, in the same list.
  await search.fill("すし");
  await expect(picker.getByRole("button", { name: /^🍣/ })).toBeVisible();

  // Keyboard: down from the search box into the results, Enter picks.
  await search.fill(emoji.name);
  await expect(picker.getByRole("button", { name: emoji.code })).toBeVisible();
  await search.press("ArrowDown");
  await expect(picker.getByRole("button", { name: emoji.code })).toBeFocused();
  await page.keyboard.press("Enter");

  await page.keyboard.press("Escape");
  await expect(picker).toBeHidden();
  await expect(page.locator(`[data-testid="deck-item"][data-emoji="${emoji.code}"]`)).toBeVisible();
  await expect.poll(() => deckOf(request, me)).toContain(emoji.code);
});

// ───────────────────────────── Emoji in text ─────────────────────────────

test("text shows a shortcode as its emoji and leaves an unknown one, Unicode and markup alone", async ({
  page,
  request,
  me: _me,
}) => {
  const emoji = await catalogEmoji(request, { label: "inu", aliases: ["わんこ"] });
  const other = await catalogEmoji(request, { label: "tori", color: [60, 120, 240] });

  await page.goto(`/emoji/${emoji.name}`);
  await expect(page.getByRole("heading", { name: emoji.code, level: 1 })).toBeVisible();
  const preview = page.getByTestId("emoji-text-preview").first();
  // The page's sample sentence already carries the emoji.
  await expect(preview.locator(`img[alt="${emoji.code}"]`)).toBeVisible();

  const field = page.getByRole("textbox", { name: "ためし書きのテキスト" });
  await field.fill(`おはよう ${emoji.code} と :nai_${emoji.name}: 😀 <b>ふとじ</b> 12:30:45`);
  await expect(preview.locator("img")).toHaveCount(1);
  await expect(preview.locator(`img[alt="${emoji.code}"]`)).toBeVisible();
  // Everything that is no known emoji stays exactly as typed, as text.
  await expect(preview).toHaveText(`おはよう  と :nai_${emoji.name}: 😀 <b>ふとじ</b> 12:30:45`);
  await expect(preview.locator("b")).toHaveCount(0);

  // `:` autocomplete: a few letters of another emoji's name suggest it; picking writes its shortcode.
  await field.fill("");
  await field.pressSequentially(`いいね :${other.name.slice(0, 8)}`);
  await page
    .getByRole("list", { name: "絵文字の候補" })
    .getByRole("button", { name: other.code })
    .click();
  await expect(field).toHaveValue(`いいね ${other.code} `);
  await expect(preview.locator(`img[alt="${other.code}"]`)).toBeVisible();
});

test("an animated emoji moves only while it is on screen, and stands still for people who asked for less motion", async ({
  page,
  request,
  me: _me,
}) => {
  const admin = await adminAccount(request);
  const name = unique("blink");
  leftovers.emoji.add(name);
  const assetId = await uploadPicture(request, admin, blinkingGif(`${name}.gif`));
  const added = await rpc<{ emoji: { emoji: { animated?: boolean; image: { url: string } } } }>(
    request,
    `${ADMIN}/AddEmoji`,
    { name, assetId },
    admin,
  );
  expect(added.emoji.emoji.animated).toBe(true);
  const moving = added.emoji.emoji.image.url;

  // A short window, so the page scrolls and its first picture can leave the screen.
  await page.setViewportSize({ width: page.viewportSize()?.width ?? 390, height: 480 });
  await page.goto(`/emoji/${name}`);
  const picture = page.getByTestId("preview-light").first().locator(`img[data-emoji="${name}"]`);
  await expect(picture).toHaveAttribute("src", moving);
  await expect(picture).toHaveAttribute("data-playing", "");

  // Scrolled away, it is swapped for its still picture.
  const viewport = page.viewportSize() ?? { width: 390, height: 480 };
  await page.mouse.move(viewport.width / 2, viewport.height / 2);
  await page.mouse.wheel(0, 5_000);
  await expect(picture).not.toHaveAttribute("data-playing", "");
  await expect(picture).not.toHaveAttribute("src", moving);
  await page.mouse.wheel(0, -10_000);
  await expect(picture).toHaveAttribute("src", moving);

  // "Reduce motion" keeps it still even on screen.
  await page.emulateMedia({ reducedMotion: "reduce" });
  await expect(picture).not.toHaveAttribute("data-playing", "");
  await expect(picture).not.toHaveAttribute("src", moving);
});

test("a sensitive emoji is blurred on its own page too, until the visitor taps to see it", async ({
  page,
  request,
  me: _me,
}) => {
  const admin = await adminAccount(request);
  const name = unique("spicy");
  leftovers.emoji.add(name);
  const assetId = await uploadPicture(request, admin, discPicture(`${name}.png`));
  await rpc(request, `${ADMIN}/AddEmoji`, { name, assetId, sensitive: true }, admin);

  await page.goto(`/emoji/${name}`);
  const picture = page.getByTestId("preview-light").first().locator(`img[data-emoji="${name}"]`);
  await expect(picture).toBeVisible();
  await expect(picture).toHaveClass(/blur/);
  await page.getByRole("button", { name: "タップして表示する" }).click();
  await expect(picture).not.toHaveClass(/blur/);
  await page.getByRole("button", { name: "またぼかす" }).click();
  await expect(picture).toHaveClass(/blur/);
});

// ───────────────────────────── Submitting and reviewing ─────────────────────────────

test("a submitted emoji is approved by a moderator and reaches another account's catalogue without a reload", async ({
  page,
  request,
  browser,
  me: _me,
}, testInfo) => {
  // The catalogue has at least one emoji, so the other account starts from a real version.
  await catalogEmoji(request, { label: "base" });
  const admin = await adminAccount(request);
  const viewer = await createAccount(request, "viewer");
  const name = unique("made");
  leftovers.emoji.add(name);
  const code = `:${name}:`;

  // Someone else has the catalogue open the whole time.
  const viewerPage = await browserOf(browser, testInfo, viewer);
  const catalogCalls: string[] = [];
  viewerPage.on("request", (sent) => {
    const match = /EmojiService\/(GetCatalog(?:Changes)?)$/.exec(sent.url());
    if (match) catalogCalls.push(match[1] as string);
  });
  await viewerPage.goto("/emoji");
  await expect(viewerPage.getByTestId("emoji-tile").first()).toBeVisible();
  await expect(viewerPage.getByRole("link", { name: code })).toHaveCount(0);

  // The author submits through the form.
  await page.goto("/emoji/new-emoji");
  // Something that is no picture is turned away before anything is uploaded.
  const uploads: string[] = [];
  page.on(
    "request",
    (sent) => sent.url().endsWith("MediaService/CreateUpload") && uploads.push(sent.url()),
  );
  await page.getByTestId("emoji-file").setInputFiles({
    name: "notes.txt",
    mimeType: "text/plain",
    buffer: Buffer.from("not a picture"),
  });
  await expect(
    page.getByRole("alert").filter({ hasText: "画像ファイルをえらんでね" }),
  ).toBeVisible();
  expect(uploads).toHaveLength(0);
  await page.getByTestId("emoji-file").setInputFiles(discPicture("made.png", [255, 170, 0]));
  // The picture is previewed at text size on a light and on a dark tile.
  await expect(
    page.getByTestId("preview-light").getByTestId("emoji-preview-line").locator("img"),
  ).toBeVisible();
  await expect(
    page.getByTestId("preview-dark").getByTestId("emoji-preview-line").locator("img"),
  ).toBeVisible();
  const nameField = page.getByRole("textbox", { name: "なまえ" });
  // A reserved name is refused while typing.
  await nameField.fill("admin");
  await expect(page.getByRole("alert").filter({ hasText: "そのなまえは使えないよ" })).toBeVisible();
  await nameField.fill(name);
  await expect(page.getByText(`${code} で使えるよ！`)).toBeVisible();
  const aliases = page.getByRole("textbox", { name: "エイリアス（検索ワード）" });
  await aliases.fill("てすと");
  await aliases.press("Enter");
  await aliases.fill("made");
  await aliases.press("Enter");
  await page.getByRole("textbox", { name: "ライセンス・クレジット" }).fill("自作です");
  await page.getByRole("button", { name: "申請する" }).click();

  // It waits for review under "my submissions".
  await expect(page).toHaveURL(/\/emoji\/submissions$/);
  const submission = page.getByTestId("submission").filter({ hasText: code });
  await expect(submission.locator('[data-state="pending"]')).toBeVisible();

  // A moderator finds it in the review queue and approves it.
  const adminPage = await browserOf(browser, testInfo, admin);
  await adminPage.goto("/admin/emoji");
  const card = adminPage.getByRole("article", { name: code });
  await expect(async () => {
    // The queue is shared with other tests: older submissions may fill the first pages.
    await adminPage.mouse.wheel(0, 20_000);
    await expect(card).toBeVisible({ timeout: 2_000 });
  }).toPass({ timeout: 30_000 });
  await expect(card).toContainText("てすと");
  await expect(card).toContainText("自作です");
  await card.getByRole("button", { name: "承認" }).click();
  await expect(adminPage.getByText(`${code} を承認しました`)).toBeVisible();
  await expect(card).toHaveCount(0);

  // The other account sees it appear, without loading the page (or the whole set) again.
  await expect(viewerPage.getByRole("link", { name: code })).toBeVisible({ timeout: 20_000 });
  expect(catalogCalls).toContain("GetCatalogChanges");
  expect(catalogCalls.filter((call) => call === "GetCatalog").length).toBeLessThanOrEqual(1);
  // And can use it right away: the picker knows it by its alias.
  await viewerPage.goto("/emoji/my-deck");
  await viewerPage.getByRole("button", { name: "追加" }).click();
  await viewerPage.getByTestId("emoji-picker").getByRole("searchbox").fill(name);
  await expect(
    viewerPage.getByTestId("emoji-picker").getByRole("button", { name: code }),
  ).toBeVisible();

  // The author is told live: the submission is now in the catalogue.
  await expect(submission.locator('[data-state="approved"]')).toBeVisible({ timeout: 20_000 });
  await submission.getByRole("link", { name: code }).click();
  await expect(page.getByRole("heading", { name: code, level: 1 })).toBeVisible();
  await expect(page.getByText("自作です")).toBeVisible();

  await viewerPage.context().close();
  await adminPage.context().close();
});

test("a declined submission shows the reason and can be fixed, resubmitted and withdrawn", async ({
  page,
  request,
  me,
}) => {
  const admin = await adminAccount(request);
  const name = unique("fixme");
  leftovers.emoji.add(name).add(`${name}_v2`);
  const assetId = await uploadPicture(request, me, discPicture("fixme.png"));
  const submitted = await rpc<{ emoji: { emoji: { id: string } } }>(
    request,
    `${SUBMISSIONS}/SubmitEmoji`,
    { name, assetId, aliases: ["なおす"] },
    me,
  );
  await rpc(
    request,
    `${ADMIN}/RejectEmoji`,
    { emojiId: submitted.emoji.emoji.id, reason: "背景を透明にしてください" },
    admin,
  );

  await page.goto("/emoji/submissions");
  const submission = page.getByTestId("submission").filter({ hasText: `:${name}:` });
  await expect(submission.locator('[data-state="rejected"]')).toBeVisible();
  await expect(submission).toContainText("背景を透明にしてください");

  await submission.getByRole("link", { name: "なおして再申請" }).click();
  await expect(page.getByText("こんかいは見送りになりました")).toBeVisible();
  const renamed = `${name}_v2`;
  await page.getByRole("textbox", { name: "なまえ" }).fill(renamed);
  await expect(page.getByText(`:${renamed}: で使えるよ！`)).toBeVisible();
  await page.getByRole("button", { name: "もういちど申請する" }).click();

  await expect(page).toHaveURL(/\/emoji\/submissions$/);
  const again = page.getByTestId("submission").filter({ hasText: `:${renamed}:` });
  await expect(again.locator('[data-state="pending"]')).toBeVisible();

  await again.getByRole("button", { name: "とりさげる" }).click();
  await page.getByRole("dialog").getByRole("button", { name: "とりさげる" }).click();
  await expect(again).toHaveCount(0);
  const mine = await rpc<{ emojis?: unknown[] }>(
    request,
    `${SUBMISSIONS}/ListMySubmissions`,
    {},
    me,
  );
  expect(mine.emojis ?? []).toHaveLength(0);
});

// ───────────────────────────── The reaction deck ─────────────────────────────

test("an emoji is pinned to the deck, moved to the front by dragging, moved with the keyboard and removed", async ({
  page,
  request,
  me,
}) => {
  const emoji = await catalogEmoji(request, { label: "pin" });

  await page.goto(`/emoji/${emoji.name}`);
  await page.getByRole("button", { name: "デッキにピンどめ" }).click();
  await expect(page.getByRole("button", { name: "デッキからはずす" })).toBeVisible();

  await page.goto("/emoji/my-deck");
  const items = page.getByTestId("deck-item");
  const codes = () =>
    items.evaluateAll((nodes) => nodes.map((node) => node.getAttribute("data-emoji")));
  // The server's default deck, then the pin.
  await expect(items).toHaveCount(11);
  expect((await codes()).at(-1)).toBe(emoji.code);
  const before = await deckOf(request, me);

  // Drag the last tile onto the first.
  const from = await items.last().boundingBox();
  const to = await items.first().boundingBox();
  if (!from || !to) throw new Error("deck tiles are not on screen");
  await page.mouse.move(from.x + from.width / 2, from.y + from.height / 2);
  await page.mouse.down();
  await page.mouse.move(from.x + from.width / 2 - 12, from.y + from.height / 2 - 6, { steps: 3 });
  await page.mouse.move(to.x + to.width / 2, to.y + to.height / 2, { steps: 12 });
  // The others make room while the tile is over them.
  await page.waitForTimeout(250);
  await page.mouse.up();
  await expect.poll(codes).toEqual([emoji.code, ...before.slice(0, -1)]);
  await expect.poll(() => deckOf(request, me)).toEqual([emoji.code, ...before.slice(0, -1)]);

  // The arrow keys move a focused tile one place.
  await items.first().focus();
  await page.keyboard.press("ArrowRight");
  await expect
    .poll(() => deckOf(request, me))
    .toEqual([before[0], emoji.code, ...before.slice(1, -1)]);

  // Take it off again.
  await page.getByRole("button", { name: `${emoji.code} をはずす` }).click();
  await expect(items).toHaveCount(10);
  await expect.poll(() => deckOf(request, me)).toEqual(before.slice(0, -1));

  // Back to the suggested deck.
  await page.getByRole("button", { name: "はじめの状態にもどす" }).click();
  await page.getByRole("dialog").getByRole("button", { name: "はじめの状態にもどす" }).click();
  await expect(page.getByText("おすすめのまま")).toBeVisible();
});

// ───────────────────────────── Staff ─────────────────────────────

test("a moderator imports a Misskey pack and gets a report; the emoji are in the catalogue", async ({
  page,
  request,
  browser,
  me: _me,
}, testInfo) => {
  const admin = await adminAccount(request);
  const taken = await catalogEmoji(request, { label: "taken" });
  const first = unique("pack_a");
  const second = unique("pack_b");
  const category = unique("ぱっく");
  leftovers.emoji.add(first).add(second);
  leftovers.categories.add(category);
  const pack = misskeyPack([
    { name: first, file: discPicture("a.png", [40, 180, 120]), category, aliases: ["いんぽーと"] },
    { name: second, file: discPicture("b.png", [150, 80, 220]), category },
    // Its name is taken: with the default policy the entry is skipped.
    { name: taken.name, file: discPicture("c.png"), category },
  ]);

  const adminPage = await browserOf(browser, testInfo, admin);
  await adminPage.goto("/admin/emoji?tab=packs");
  await adminPage.getByTestId("import-file").setInputFiles(pack);
  await expect(adminPage.getByText("emoji-pack.zip")).toBeVisible();
  await adminPage.getByRole("button", { name: "インポートをはじめる" }).click();

  const report = adminPage.getByTestId("import-report");
  await expect(report).toBeVisible({ timeout: 30_000 });
  await expect(report.getByRole("progressbar")).toBeVisible();
  await expect(report).toHaveAttribute("data-status", "completed", { timeout: 90_000 });
  await expect(report.getByTestId("import-summary")).toHaveText(
    "3個中：とりこみ 2・スキップ 1・失敗 0",
  );
  await expect(report.getByText("misskey.e2e.example からのパック")).toBeVisible();
  await expect(report.locator('[data-testid="import-entry"][data-status="imported"]')).toHaveCount(
    2,
  );
  const skipped = report.locator('[data-testid="import-entry"][data-status="skipped"]');
  await expect(skipped).toContainText(taken.name);
  await expect(skipped).toContainText("おなじなまえの絵文字があります");

  // The imported emoji are live, in the pack's category, for everyone.
  await page.goto(`/emoji?category=${encodeURIComponent(category)}`);
  await expect(page.getByRole("link", { name: `:${first}:` })).toBeVisible({ timeout: 20_000 });
  await expect(page.getByRole("link", { name: `:${second}:` })).toBeVisible();
  await expect(page.getByTestId("emoji-tile")).toHaveCount(2);

  // And the set goes out again as a ZIP.
  await adminPage.getByRole("button", { name: "エクスポート", exact: true }).click();
  const exported = adminPage.getByTestId("export-row").first();
  await expect(exported).toHaveAttribute("data-status", "ready", { timeout: 90_000 });
  await expect(exported.getByRole("button", { name: "ダウンロード" })).toBeVisible();

  await adminPage.context().close();
});

test("a moderator edits, disables, enables and deletes an emoji, and trusts a submitter", async ({
  request,
  browser,
}, testInfo) => {
  const admin = await adminAccount(request);
  const emoji = await catalogEmoji(request, { label: "staff" });
  const maker = await createAccount(request, "maker");
  const adminPage = await browserOf(browser, testInfo, admin);
  const stateOf = async () =>
    (
      await rpc<{ emojis?: { state: string; emoji: { aliases?: string[] } }[] }>(
        request,
        `${ADMIN}/ListEmoji`,
        { query: emoji.name },
        admin,
      )
    ).emojis?.[0];

  await adminPage.goto(`/admin/emoji?tab=all&q=${emoji.name}`);
  const row = adminPage.locator(`[data-testid="admin-emoji"][data-name="${emoji.name}"]`);
  await expect(row).toBeVisible();
  const menu = row.getByRole("button", { name: `${emoji.code} の操作` });

  // Edit: add an alias.
  await menu.click();
  await adminPage.getByRole("menuitem", { name: "編集" }).click();
  const dialog = adminPage.getByRole("dialog");
  const aliases = dialog.getByRole("textbox", { name: "エイリアス（検索ワード）" });
  await aliases.fill("かんり");
  await aliases.press("Enter");
  await dialog.getByRole("button", { name: "保存" }).click();
  await expect(adminPage.getByText(`${emoji.code} を保存しました`)).toBeVisible();
  await expect.poll(async () => (await stateOf())?.emoji.aliases).toEqual(["かんり"]);

  // Disable with a reason, then enable again.
  await menu.click();
  await adminPage.getByRole("menuitem", { name: "停止する" }).click();
  await adminPage.getByRole("dialog").getByRole("textbox", { name: "理由" }).fill("テストのため");
  await adminPage.getByRole("dialog").getByRole("button", { name: "停止する" }).click();
  await expect(row.locator('[data-state="disabled"]')).toBeVisible();
  await menu.click();
  await adminPage.getByRole("menuitem", { name: "もどす" }).click();
  await expect(row.locator('[data-state="approved"]')).toBeVisible();

  // Delete for good.
  await menu.click();
  await adminPage.getByRole("menuitem", { name: "削除" }).click();
  await adminPage.getByRole("dialog").getByRole("button", { name: "削除" }).click();
  await expect(row).toHaveCount(0);
  await expect.poll(stateOf).toBeUndefined();

  // Trusted submitters: add one by handle; their emoji then skip the review.
  await adminPage.goto("/admin/emoji?tab=trusted");
  await adminPage.getByRole("textbox", { name: "信頼ユーザーを追加" }).fill(maker.handle);
  await adminPage.getByRole("button", { name: new RegExp(`@${maker.handle}`) }).click();
  await expect(
    adminPage.getByTestId("trusted-row").filter({ hasText: `@${maker.handle}` }),
  ).toBeVisible();
  const profile = await rpc<{ profile: { trusted?: boolean } }>(
    request,
    `${SUBMISSIONS}/GetSubmitterProfile`,
    {},
    maker,
  );
  expect(profile.profile.trusted).toBe(true);
  await adminPage
    .getByTestId("trusted-row")
    .filter({ hasText: `@${maker.handle}` })
    .getByRole("button", { name: "はずす" })
    .click();
  await expect(
    adminPage.getByTestId("trusted-row").filter({ hasText: `@${maker.handle}` }),
  ).toHaveCount(0);

  await adminPage.context().close();
});

test("the catalogue page filters by category and search, and someone without a staff role cannot open the staff page", async ({
  page,
  request,
  me: _me,
}) => {
  const tag = unique("s").replace(/_/g, "");
  const emoji = await catalogEmoji(request, { label: "find", aliases: [`さがす${tag}`] });

  await page.goto("/emoji");
  await page.getByRole("searchbox").fill(`サガス${tag}`);
  await expect(page.getByTestId("emoji-tile")).toHaveCount(1);
  await expect(page.getByRole("link", { name: emoji.code })).toBeVisible();
  await expect(page).toHaveURL(/q=/);
  await page.getByRole("link", { name: emoji.code }).click();
  await expect(page.getByRole("heading", { name: emoji.code, level: 1 })).toBeVisible();
  await expect(page.getByText(`さがす${tag}`)).toBeVisible();
  await page.getByRole("button", { name: `${emoji.code} をコピー` }).click();
  await expect(page.getByText(`${emoji.code} をコピーしたよ！`)).toBeVisible();

  await page.goto("/admin/emoji");
  await expect(page.getByRole("button", { name: "承認" })).toHaveCount(0);
  await expect(page.getByRole("tab", { name: "審査" })).toHaveCount(0);
});

// ───────────────────────────── Emoji across the app ─────────────────────────────

test("a custom emoji is drawn in a chat message and in a note, and `:` suggests it while writing", async ({
  page,
  request,
  me,
}) => {
  const emoji = await catalogEmoji(request, { label: "kani", color: [240, 120, 40] });
  const friend = await createAccount(request, "friend");
  await befriend(request, me, friend);

  // A message a friend sent with the shortcode shows the picture, in the conversation.
  const { conversation } = await rpc<{ conversation: { id: string } }>(
    request,
    "tokuly.chat.v1.ConversationService/OpenDirect",
    { userId: me.userId },
    friend,
  );
  await rpc(
    request,
    "tokuly.chat.v1.MessageService/SendMessage",
    {
      conversationId: conversation.id,
      clientId: crypto.randomUUID(),
      text: `やっほー ${emoji.code}`,
    },
    friend,
  );
  await page.goto(`/chat/${conversation.id}`);
  const message = page.locator('[data-testid="message"]', { hasText: "やっほー" });
  await expect(message.locator(`img[alt="${emoji.code}"]`)).toBeVisible();

  // Typing a colon and the start of the name offers it; a tap writes the shortcode.
  const composer = page.getByRole("textbox", { name: "メッセージを入力" });
  await composer.pressSequentially(`おつかれ :${emoji.name.slice(0, 8)}`);
  await page
    .getByRole("list", { name: "絵文字の候補" })
    .getByRole("button", { name: emoji.code })
    .click();
  await expect(composer).toHaveValue(`おつかれ ${emoji.code} `);
  await composer.press("Enter");
  // On a phone Enter is a new line: the send button sends.
  const send = page.getByRole("button", { name: "送信", exact: true });
  if (await send.isVisible().catch(() => false)) await send.click();
  await expect(
    page
      .locator('[data-testid="message"]', { hasText: "おつかれ" })
      .locator(`img[alt="${emoji.code}"]`),
  ).toBeVisible();

  // The same emoji in a note.
  const NOTES = "tokuly.notes.v1.NoteService";
  const rules = await rpc<{ version: number }>(request, `${NOTES}/GetNoteRules`, {}, friend);
  await rpc(request, `${NOTES}/AgreeToNoteRules`, { version: rules.version }, friend);
  const text = `かにだよ ${unique("note")}`;
  const created = await rpc<{ note: { id: string } }>(
    request,
    `${NOTES}/CreateNote`,
    { text: `${text} ${emoji.code}` },
    friend,
  );
  await page.goto(`/notes/${created.note.id}`);
  const note = page.locator('[data-testid="note"]', { hasText: text });
  await expect(note.locator(`img[alt="${emoji.code}"]`)).toBeVisible();
});
