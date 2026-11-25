import zlib from "node:zlib";
import type { APIRequestContext, Locator, Page } from "@playwright/test";
import { type Account, adminAccount, createAccount, expect, rpc, signInAs, test } from "./support";

// These tests upload pictures and wait for the server to process them, several times each.
test.describe.configure({ timeout: 90_000 });

const STORE = "tokuly.stamp.v1.StampStoreService";
const LIBRARY = "tokuly.stamp.v1.StampLibraryService";
const STUDIO = "tokuly.stamp.v1.StampStudioService";
const REVIEW = "tokuly.stamp.v1.StampReviewService";
const MEDIA = "tokuly.media.v1.MediaService";
const WALLET = "tokuly.wallet.v1.WalletService";

// ───────────────────────────── Files made for the tests ─────────────────────────────

interface TestFile {
  name: string;
  mimeType: string;
  buffer: Buffer;
}

type Rgba = [number, number, number, number];
const WHITE: Rgba = [255, 255, 255, 255];
const RED: Rgba = [230, 40, 60, 255];
const BLUE: Rgba = [60, 120, 240, 255];

/** A PNG picture: a coloured disc in the middle of a plain background (white unless said otherwise). */
function discPicture(
  name: string,
  color: Rgba = RED,
  background: Rgba = WHITE,
  size = 240,
): TestFile {
  const row = size * 4 + 1;
  const raw = Buffer.alloc(row * size);
  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      const inside = Math.hypot(x - size / 2, y - size / 2) < size * 0.3;
      raw.set(inside ? color : background, y * row + 1 + x * 4);
    }
  }
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
  // 8 bits per channel, RGBA.
  header.set([8, 6, 0, 0, 0], 8);
  const signature = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
  return {
    name,
    mimeType: "image/png",
    buffer: Buffer.concat([
      signature,
      chunk("IHDR", header),
      chunk("IDAT", zlib.deflateSync(raw)),
      chunk("IEND", Buffer.alloc(0)),
    ]),
  };
}

/** A ZIP archive of the given files (deflated), as dotLottie tools write them. */
function zipArchive(files: Record<string, string>): Buffer {
  const locals: Buffer[] = [];
  const directory: Buffer[] = [];
  let offset = 0;
  for (const [name, text] of Object.entries(files)) {
    const data = Buffer.from(text);
    const packed = zlib.deflateRawSync(data);
    const fileName = Buffer.from(name);
    const local = Buffer.alloc(30);
    local.writeUInt32LE(0x04034b50, 0);
    local.writeUInt16LE(20, 4);
    local.writeUInt16LE(8, 8);
    local.writeUInt32LE(zlib.crc32(data) >>> 0, 14);
    local.writeUInt32LE(packed.length, 18);
    local.writeUInt32LE(data.length, 22);
    local.writeUInt16LE(fileName.length, 26);
    const central = Buffer.alloc(46);
    central.writeUInt32LE(0x02014b50, 0);
    central.writeUInt16LE(20, 4);
    central.writeUInt16LE(20, 6);
    central.writeUInt16LE(8, 10);
    central.writeUInt32LE(zlib.crc32(data) >>> 0, 16);
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

/** A dotLottie animation: a red disc that pulses, two seconds long. */
function pulsingLottie(name: string): TestFile {
  const animation = {
    v: "5.9.0",
    fr: 30,
    ip: 0,
    op: 60,
    w: 200,
    h: 200,
    layers: [
      {
        ty: 4,
        ind: 1,
        ip: 0,
        op: 60,
        st: 0,
        ks: {
          o: { a: 0, k: 100 },
          p: { a: 0, k: [100, 100] },
          a: { a: 0, k: [0, 0] },
          r: { a: 0, k: 0 },
          s: {
            a: 1,
            k: [
              { t: 0, s: [70, 70], i: { x: [0.4], y: [1] }, o: { x: [0.6], y: [0] } },
              { t: 30, s: [100, 100], i: { x: [0.4], y: [1] }, o: { x: [0.6], y: [0] } },
              { t: 59, s: [70, 70] },
            ],
          },
        },
        shapes: [
          {
            ty: "gr",
            it: [
              { ty: "el", p: { a: 0, k: [0, 0] }, s: { a: 0, k: [150, 150] } },
              { ty: "fl", c: { a: 0, k: [0.9, 0.1, 0.2, 1] }, o: { a: 0, k: 100 }, r: 1 },
              {
                ty: "tr",
                p: { a: 0, k: [0, 0] },
                a: { a: 0, k: [0, 0] },
                s: { a: 0, k: [100, 100] },
                r: { a: 0, k: 0 },
                o: { a: 0, k: 100 },
              },
            ],
          },
        ],
      },
    ],
  };
  const manifest = { version: "1", generator: "tokuly-e2e", animations: [{ id: "pulse" }] };
  return {
    name,
    mimeType: "application/zip",
    buffer: zipArchive({
      "manifest.json": JSON.stringify(manifest),
      "animations/pulse.json": JSON.stringify(animation),
    }),
  };
}

// ───────────────────────────── Arranging state through the API ─────────────────────────────

interface PackInfo {
  id: string;
  shortId: string;
  name: string;
  creator: Account;
  stampIds: string[];
}

interface PackResponse {
  pack: {
    summary: { id: string; shortId: string; status: string };
    stamps?: { stampRef: { id: string; name?: string } }[];
  };
}

let counter = 0;
/** A name no other test (or earlier run) uses, so lists and searches can be asserted on. */
const unique = (label: string) =>
  `${label}${Date.now().toString(36).slice(-5)}${(counter++).toString(36)}`;

/** Uploads a file as a stamp asset and waits until the server has processed it. */
async function uploadStampAsset(
  request: APIRequestContext,
  as: Account,
  file: TestFile,
): Promise<string> {
  const created = await rpc<{
    assetId: string;
    uploadId: string;
    putUrl: string;
    putHeaders?: Record<string, string>;
  }>(
    request,
    `${MEDIA}/CreateUpload`,
    {
      purpose: "ASSET_PURPOSE_STAMP",
      fileName: file.name,
      contentType: file.mimeType,
      sizeBytes: String(file.buffer.length),
    },
    as,
  );
  const put = await request.put(created.putUrl, {
    data: file.buffer,
    headers: created.putHeaders ?? {},
  });
  expect(put.ok(), `upload of ${file.name}`).toBeTruthy();
  await rpc(request, `${MEDIA}/CompleteUpload`, { uploadId: created.uploadId }, as);
  await expect(async () => {
    const { asset } = await rpc<{ asset: { status: string } }>(
      request,
      `${MEDIA}/GetAsset`,
      { assetId: created.assetId },
      as,
    );
    expect(asset.status).toBe("ASSET_STATUS_READY");
  }).toPass({ timeout: 30_000 });
  return created.assetId;
}

/** A draft pack of `creator` with stamps named "<name>1", "<name>2"… */
async function draftPack(
  request: APIRequestContext,
  creator: Account,
  options: { name?: string; price?: number; stamps?: number } = {},
): Promise<PackInfo> {
  const name = options.name ?? unique("ぱっく");
  const created = await rpc<PackResponse>(
    request,
    `${STUDIO}/CreatePack`,
    { name, description: "e2e のパックです", tags: ["e2e"], price: options.price ?? 0 },
    creator,
  );
  const stamps = [];
  for (let index = 0; index < (options.stamps ?? 2); index++) {
    const assetId = await uploadStampAsset(
      request,
      creator,
      discPicture(`s${index}.png`, index % 2 === 0 ? RED : BLUE, [0, 0, 0, 0]),
    );
    stamps.push({ assetId, name: `${name}${index + 1}` });
  }
  const added =
    stamps.length > 0
      ? await rpc<PackResponse>(
          request,
          `${STUDIO}/AddStamps`,
          { packId: created.pack.summary.id, stamps },
          creator,
        )
      : created;
  return {
    id: created.pack.summary.id,
    shortId: created.pack.summary.shortId,
    name,
    creator,
    stampIds: (added.pack.stamps ?? []).map((item) => item.stampRef.id),
  };
}

/** A pack waiting in the review queue. */
async function submittedPack(
  request: APIRequestContext,
  options: { name?: string; price?: number; stamps?: number } = {},
): Promise<PackInfo> {
  const pack = await draftPack(request, await createAccount(request, "creator"), options);
  await rpc(request, `${STUDIO}/SubmitPack`, { packId: pack.id }, pack.creator);
  return pack;
}

/** A pack that went through review and is in the store. */
async function publishedPack(
  request: APIRequestContext,
  options: { name?: string; price?: number; stamps?: number } = {},
): Promise<PackInfo> {
  const pack = await submittedPack(request, options);
  await rpc(request, `${REVIEW}/ApprovePack`, { packId: pack.id }, await adminAccount(request));
  return pack;
}

/** Takes a pack a test left waiting out of the shared review queue. */
async function withdraw(request: APIRequestContext, pack: PackInfo): Promise<void> {
  await rpc(request, `${STUDIO}/UnpublishPack`, { packId: pack.id }, pack.creator).catch(() => {});
}

/** Buys the 100-coin package with the development payment provider. */
async function grantCoins(request: APIRequestContext, to: Account): Promise<void> {
  const shop = await rpc<{ packages: { id: string; sku: string }[] }>(
    request,
    `${WALLET}/ListCoinPackages`,
    {},
    to,
  );
  const hundred = shop.packages.find((candidate) => candidate.sku === "coins_100");
  expect(hundred, "the seeded coins_100 package").toBeTruthy();
  await rpc(request, `${WALLET}/StartPurchase`, { packageId: hundred?.id }, to);
}

async function coinsOf(request: APIRequestContext, account: Account): Promise<string> {
  return (
    (await rpc<{ balance: { coins?: string } }>(request, `${WALLET}/GetBalance`, {}, account))
      .balance.coins ?? "0"
  );
}

// ───────────────────────────── Things on screen ─────────────────────────────

/**
 * Opens a page of the app and waits for the signed-in frame. The first
 * visit after the dev server changed anything loads hundreds of modules, far
 * longer than an ordinary assertion waits.
 */
async function open(page: Page, url: string): Promise<void> {
  await page.goto(url);
  await expect(page.getByRole("main")).toBeVisible({ timeout: 45_000 });
}

/** An amount as the `Coins` glyph writes it: the coin's "T", then the number. */
const coins = (amount: number) => new RegExp(`^T${amount.toLocaleString("en-US")}$`);

const stampTiles = (scope: Page | Locator) => scope.getByTestId("stamp-tile");

/** Walks the cut-out editor from a chosen picture to the naming step, removing the background on the way. */
async function cutOut(page: Page, file: TestFile): Promise<void> {
  await page.locator('input[type="file"][accept="image/*"]').setInputFiles(file);
  const dialog = page.getByRole("dialog", { name: "切り抜く" });
  await expect(dialog.getByTestId("cutout-stage")).toBeVisible();
  await dialog.getByRole("button", { name: "背景を消す" }).click();
  // The removal is one undoable step.
  await expect(dialog.getByRole("button", { name: "もとに戻す" })).toBeEnabled();
  await dialog.getByRole("button", { name: "つぎへ" }).click();
}

/** Reads a picture's size and a few pixels in the browser: `[r, g, b, a]` at each asked position (0..1 of the size). */
async function readPixels(
  page: Page,
  url: string,
  points: [number, number][],
): Promise<{ width: number; height: number; pixels: number[][] }> {
  return page.evaluate(
    async ({ url, points }) => {
      const image = new Image();
      image.src = url;
      await image.decode();
      const canvas = document.createElement("canvas");
      canvas.width = image.naturalWidth;
      canvas.height = image.naturalHeight;
      const context = canvas.getContext("2d");
      if (!context) throw new Error("no canvas");
      context.drawImage(image, 0, 0);
      return {
        width: canvas.width,
        height: canvas.height,
        pixels: points.map(([x, y]) => [
          ...context.getImageData(
            Math.floor(x * (canvas.width - 1)),
            Math.floor(y * (canvas.height - 1)),
            1,
            1,
          ).data,
        ]),
      };
    },
    { url, points },
  );
}

// ───────────────────────────── Store and library ─────────────────────────────

test.describe("store", () => {
  test("a free pack is added from its page, shows up in the picker, and can be removed again", async ({
    page,
    request,
    me: _me,
  }) => {
    const pack = await publishedPack(request, { stamps: 3 });

    await open(page, `/stamps/${pack.shortId}`);
    await expect(page.getByRole("heading", { name: pack.name, level: 2 })).toBeVisible();
    await expect(page.getByText("e2e のパックです")).toBeVisible();
    await expect(stampTiles(page)).toHaveCount(3);

    await page.getByRole("button", { name: "無料で追加する" }).click();
    await expect(page.getByText("ピッカーからすぐ使えるよ！")).toBeVisible();

    // The picker has a tab for the pack; picking one of its stamps hands it over (here: opens its details).
    await open(page, "/stamps/library");
    await expect(page.getByTestId("installed-pack").filter({ hasText: pack.name })).toBeVisible();
    await page.getByRole("button", { name: "ピッカー" }).click();
    const picker = page.getByTestId("stamp-picker");
    await picker.getByRole("tab", { name: pack.name }).click();
    await expect(picker.getByTestId("picker-stamp")).toHaveCount(3);
    await picker.getByRole("button", { name: `${pack.name}1` }).click();
    await expect(picker).toHaveCount(0);
    await expect(page.getByRole("dialog", { name: `${pack.name}1` })).toBeVisible();
    await page.keyboard.press("Escape");

    // Removing it takes it out of the library; the pack page offers it again.
    await open(page, `/stamps/${pack.shortId}`);
    await page.getByRole("button", { name: "はずす" }).click();
    await expect(page.getByRole("button", { name: "無料で追加する" })).toBeVisible();
    await open(page, "/stamps/library");
    await expect(page.getByText("まだパックがないよ")).toBeVisible();
  });

  test("the front page has shelves, tags and a search that finds a published pack", async ({
    page,
    request,
    me: _me,
  }) => {
    const pack = await publishedPack(request, { name: unique("さがせるぱっく"), stamps: 1 });

    await open(page, "/stamps");
    await expect(page.getByRole("heading", { name: "スタンプ", level: 1 })).toBeVisible();
    await expect(
      page.getByRole("region", { name: "新着" }).getByTestId("pack-card").first(),
    ).toBeVisible();
    await expect(page.getByRole("link", { name: /マイスタンプ/ })).toHaveAttribute(
      "href",
      "/stamps/library",
    );
    await expect(page.getByRole("link", { name: /つくる/ })).toHaveAttribute(
      "href",
      "/stamps/studio",
    );

    // The browse page offers the popular tags; the one every test pack carries leads to its shelf.
    await open(page, "/stamps/browse");
    await expect(
      page.getByRole("navigation", { name: "人気のタグ" }).getByRole("link", { name: /e2e/ }),
    ).toBeVisible();
    await open(page, "/stamps/browse?tag=e2e&section=new");
    await expect(page.getByRole("heading", { name: "#e2e" })).toBeVisible();
    await expect(page.getByTestId("pack-card").filter({ hasText: pack.name })).toBeVisible();

    // The search index follows publication a moment later.
    await expect(async () => {
      await open(page, `/stamps/browse?q=${encodeURIComponent(pack.name)}`);
      await expect(page.getByTestId("pack-card").filter({ hasText: pack.name })).toBeVisible({
        timeout: 3_000,
      });
    }).toPass({ timeout: 30_000 });
    await page.getByTestId("pack-card").filter({ hasText: pack.name }).click();
    await expect(page).toHaveURL(new RegExp(`/stamps/${pack.shortId}$`));

    // The creator's own page lists it too. (Exactly the creator's link: the cards of packs that
    // other tests' creators published, also named "creator", may be on the page as well.)
    await page.getByRole("link", { name: "creator", exact: true }).click();
    await expect(page.getByRole("heading", { name: "クリエイター" })).toBeVisible();
    await expect(page.getByTestId("pack-card").filter({ hasText: pack.name })).toBeVisible();
  });

  test("a stamp is marked as a favourite from its preview and collected in the library", async ({
    page,
    request,
    me,
  }) => {
    const pack = await publishedPack(request, { stamps: 2 });
    await rpc(request, `${LIBRARY}/InstallPack`, { packId: pack.id }, me);

    await open(page, `/stamps/${pack.shortId}`);
    await page.getByRole("button", { name: `${pack.name}2` }).click();
    const preview = page.getByRole("dialog", { name: `${pack.name}2` });
    await preview.getByRole("button", { name: "お気に入りに追加" }).click();
    await expect(preview.getByRole("button", { name: "お気に入りからはずす" })).toBeVisible();
    await page.keyboard.press("Escape");
    // The tile wears the heart.
    await expect(
      stampTiles(page)
        .filter({ has: page.getByRole("button", { name: `${pack.name}2` }) })
        .getByLabel("お気に入り"),
    ).toBeVisible();

    await open(page, "/stamps/library?tab=favorites");
    await expect(stampTiles(page)).toHaveCount(1);
    await expect(page.getByRole("button", { name: `${pack.name}2` })).toBeVisible();

    // Un-marking it empties the list again.
    await page.getByRole("button", { name: `${pack.name}2` }).click();
    await page.getByRole("dialog").getByRole("button", { name: "お気に入りからはずす" }).click();
    await page.keyboard.press("Escape");
    await expect(page.getByText("お気に入りはまだないよ")).toBeVisible();
  });

  test("saving a stamp from its preview adds the pack it comes from", async ({
    page,
    request,
    me,
  }) => {
    const pack = await publishedPack(request, { stamps: 2 });

    await open(page, `/stamps/${pack.shortId}`);
    await page.getByRole("button", { name: `${pack.name}1` }).click();
    const preview = page.getByRole("dialog", { name: `${pack.name}1` });
    await preview.getByRole("button", { name: "保存する" }).click();
    await expect(page.getByText(`「${pack.name}」を追加しました！`)).toBeVisible();
    // Saved: nothing left to save, and the stamp can be a favourite now.
    await expect(preview.getByRole("button", { name: "保存する" })).toHaveCount(0);
    await expect(preview.getByRole("button", { name: "お気に入りに追加" })).toBeVisible();
    await expect
      .poll(async () =>
        (
          await rpc<{ packs?: { id: string }[] }>(request, `${LIBRARY}/ListInstalledPacks`, {}, me)
        ).packs?.map((installed) => installed.id),
      )
      .toEqual([pack.id]);
  });

  test("a paid pack is bought with coins after a confirmation, and the balance goes down", async ({
    page,
    request,
    me,
  }) => {
    const pack = await publishedPack(request, { price: 40, stamps: 1 });
    await grantCoins(request, me);

    await open(page, `/stamps/${pack.shortId}`);
    await page.getByRole("button", { name: "40コインで購入" }).click();
    const dialog = page.getByRole("dialog", { name: "コインで購入する" });
    await expect(dialog.getByText(pack.name)).toBeVisible();
    await expect(dialog.getByTestId("purchase-price").getByText(coins(40))).toBeVisible();
    await expect(dialog.getByTestId("purchase-balance").getByText(coins(100))).toBeVisible();
    await expect(dialog.getByTestId("purchase-after").getByText(coins(60))).toBeVisible();
    await dialog.getByRole("button", { name: "購入する" }).click();

    await expect(page.getByText(`「${pack.name}」をゲット！🎉`)).toBeVisible();
    await expect(page.getByText("購入済み。はずしても、また追加できるよ")).toBeVisible();
    expect(await coinsOf(request, me)).toBe("60");

    // The store's header shows what is left, and the purchase is kept in the library.
    await open(page, "/stamps");
    await expect(page.getByRole("link", { name: /コイン残高 60/ })).toBeVisible();
    await open(page, "/stamps/library?tab=purchased");
    await expect(page.getByText(pack.name)).toBeVisible();
  });

  test("someone short on coins is sent to the coin shop instead of being charged", async ({
    page,
    request,
    me,
  }) => {
    const pack = await publishedPack(request, { price: 40, stamps: 1 });

    await open(page, `/stamps/${pack.shortId}`);
    await page.getByRole("button", { name: "40コインで購入" }).click();
    const dialog = page.getByRole("dialog", { name: "コインで購入する" });
    await expect(dialog.getByRole("alert")).toHaveText("コインが40枚たりないよ…");
    await expect(dialog.getByRole("button", { name: "購入する" })).toHaveCount(0);
    await expect(dialog.getByRole("link", { name: "コインをチャージ" })).toHaveAttribute(
      "href",
      "/wallet/buy",
    );
    expect(await coinsOf(request, me)).toBe("0");
  });
});

test.describe("library", () => {
  test("a personal stamp is cut out of a picture made in the test: background gone, white outline, 512 px", async ({
    page,
    request,
    me,
  }) => {
    await open(page, "/stamps/library?tab=personal");
    await expect(page.getByText("じぶんのスタンプはまだないよ")).toBeVisible();
    await page.getByRole("button", { name: "つくる", exact: true }).click();

    await cutOut(page, discPicture("わーい.png"));
    const details = page.getByRole("dialog", { name: "じぶんのスタンプに追加" });
    await expect(details.getByTestId("cutout-preview")).toBeVisible();
    // The file's name is the suggestion.
    await expect(details.getByLabel("なまえ")).toHaveValue("わーい");
    await details.getByLabel("なまえ").fill("あかまる");
    await details.getByLabel("キーワード").fill("まる あか");
    await details.getByRole("button", { name: "スタンプにする！" }).click();

    await expect(page.getByText("スタンプができた！🎉")).toBeVisible({ timeout: 30_000 });
    await expect(stampTiles(page)).toHaveCount(1);
    await expect(page.getByRole("button", { name: "あかまる" })).toBeVisible();

    // What was uploaded: a square stamp, clear in the corner, the disc in the middle, a white rim around it.
    const { stamps } = await rpc<{
      stamps: { stampRef: { imageUrl: string; name: string }; keywords?: string[] }[];
    }>(request, `${LIBRARY}/ListPersonalStamps`, {}, me);
    expect(stamps[0]?.keywords).toEqual(["まる", "あか"]);
    const picture = await readPixels(page, stamps[0]?.stampRef.imageUrl ?? "", [
      [0, 0],
      [0.5, 0.5],
      [0.5, 0.05],
    ]);
    expect([picture.width, picture.height]).toEqual([512, 512]);
    expect(picture.pixels[0]?.[3]).toBe(0);
    const [red, green, blue, alpha] = picture.pixels[1] ?? [];
    expect(alpha).toBe(255);
    expect(red).toBeGreaterThan(200);
    expect(green).toBeLessThan(90);
    expect(blue).toBeLessThan(110);
    // The disc was fitted to the frame, so its top edge is near the top: there sits the outline.
    const rim = picture.pixels[2] ?? [];
    expect(rim[3]).toBeGreaterThan(200);
    expect(Math.min(rim[0] ?? 0, rim[1] ?? 0, rim[2] ?? 0)).toBeGreaterThan(215);

    // It is in the picker at once, under "my stamps".
    await page.getByRole("button", { name: "ピッカー" }).click();
    const picker = page.getByTestId("stamp-picker");
    await picker.getByRole("tab", { name: "じぶんのスタンプ" }).click();
    await expect(picker.getByRole("button", { name: "あかまる" })).toBeVisible();
  });

  test("a personal stamp can be renamed, found by its keyword, and deleted", async ({
    page,
    request,
    me,
  }) => {
    const assetId = await uploadStampAsset(request, me, discPicture("a.png", BLUE, [0, 0, 0, 0]));
    await rpc(request, `${LIBRARY}/CreatePersonalStamp`, { assetId, name: "あおまる" }, me);

    await open(page, "/stamps/library?tab=personal");
    await page.getByRole("button", { name: "あおまる" }).click();
    const sheet = page.getByRole("dialog");
    await expect(sheet.getByText("あなたがつくったスタンプ")).toBeVisible();
    await sheet.getByRole("button", { name: "なまえを編集" }).click();
    await sheet.getByLabel("なまえ").fill("そらまる");
    await sheet.getByLabel("キーワード").fill("そら");
    await sheet.getByRole("button", { name: "保存する" }).click();
    await expect(page.getByRole("dialog", { name: "そらまる" })).toBeVisible();
    await page.keyboard.press("Escape");

    // The library search knows the new keyword.
    await page.getByRole("searchbox", { name: "持ってるスタンプをさがす" }).fill("そら");
    await expect(stampTiles(page)).toHaveCount(1);
    await page.getByRole("button", { name: "そらまる" }).click();

    await page
      .getByRole("dialog", { name: "そらまる" })
      .getByRole("button", { name: "削除する" })
      .click();
    await page
      .getByRole("dialog", { name: "このスタンプを削除する？" })
      .getByRole("button", { name: "削除する" })
      .click();
    await expect(page.getByText("削除しました")).toBeVisible();
    await page.getByRole("searchbox", { name: "持ってるスタンプをさがす" }).fill("");
    await expect(page.getByText("じぶんのスタンプはまだないよ")).toBeVisible();
  });

  test("installed packs are reordered by dragging and with the keyboard, and the order sticks", async ({
    page,
    request,
    me,
  }) => {
    // A creator's own drafts are installed for them: three packs without publishing anything.
    const names = [unique("いち"), unique("に"), unique("さん")];
    for (const name of names) await rpc(request, `${STUDIO}/CreatePack`, { name }, me);
    const rows = page.getByTestId("installed-pack");
    const order = async () =>
      (await rows.allInnerTexts()).map((text) => names.find((name) => text.includes(name)));

    await open(page, "/stamps/library");
    // New installs go first.
    await expect.poll(order).toEqual([names[2], names[1], names[0]]);

    // Drag the first pack below the last one.
    const grip = page.getByRole("button", { name: `${names[2]} をならべかえる` });
    const from = await grip.boundingBox();
    const to = await rows.nth(2).boundingBox();
    if (!from || !to) throw new Error("rows are not on screen");
    await page.mouse.move(from.x + from.width / 2, from.y + from.height / 2);
    await page.mouse.down();
    await page.mouse.move(from.x + from.width / 2, from.y + from.height / 2 + 20, { steps: 4 });
    await page.mouse.move(to.x + to.width / 2, to.y + to.height / 2, { steps: 12 });
    await expect.poll(order).toEqual([names[1], names[0], names[2]]);
    await page.mouse.up();
    await expect.poll(order).toEqual([names[1], names[0], names[2]]);

    // The arrow keys move a pack one place.
    await page.getByRole("button", { name: `${names[1]} をならべかえる` }).focus();
    await page.keyboard.press("ArrowDown");
    await expect.poll(order).toEqual([names[0], names[1], names[2]]);

    await expect
      .poll(async () =>
        (
          await rpc<{ packs: { name: string }[] }>(request, `${LIBRARY}/ListInstalledPacks`, {}, me)
        ).packs.map((pack) => pack.name),
      )
      .toEqual([names[0], names[1], names[2]]);
    await page.reload();
    await expect.poll(order).toEqual([names[0], names[1], names[2]]);
  });

  test("the history tabs explain themselves while nothing was sent yet", async ({
    page,
    me: _me,
  }) => {
    await open(page, "/stamps/library?tab=recents");
    await expect(page.getByText("まだスタンプを使ってないよ")).toBeVisible();
    await page.getByRole("tab", { name: "ログ" }).click();
    await expect(page).toHaveURL(/tab=log/);
    await expect(page.getByText("まだ何も送ってないよ")).toBeVisible();
    await page.getByRole("tab", { name: "購入済み" }).click();
    await expect(page.getByText("買ったパックはまだないよ")).toBeVisible();
  });
});

// ───────────────────────────── Studio and review ─────────────────────────────

test.describe("studio", () => {
  test("a creator makes a pack, adds finished files and a cut-out, and submits it for review", async ({
    page,
    request,
    me,
  }) => {
    const name = unique("つくったぱっく");
    await open(page, "/stamps/studio");
    await expect(page.getByText("公開には審査があります")).toBeVisible();
    await expect(page.getByText("まだパックがないよ")).toBeVisible();

    await page.getByRole("button", { name: "新しいパック" }).first().click();
    const form = page.getByRole("dialog", { name: "新しいパック" });
    await form.getByLabel("パックのなまえ").fill(name);
    await form.getByLabel("説明").fill("ブラウザテストがつくったパック");
    await form.getByLabel("タグ").fill("#E2E まる");
    await form.getByRole("button", { name: "つくる" }).click();

    await expect(page).toHaveURL(/\/stamps\/studio\/[\w-]{11}$/);
    await expect(page.getByRole("heading", { name, level: 1 })).toBeVisible();
    const status = page.getByTestId("pack-status");
    await expect(status.getByText("下書き")).toBeVisible();
    // Nothing to review yet.
    await expect(status.getByRole("button", { name: "審査に出す" })).toBeDisabled();
    await expect(page.getByText("#e2e")).toBeVisible();

    // Finished files go in as they are, named after the file.
    await page
      .locator('input[type="file"][multiple]')
      .setInputFiles([
        discPicture("にこにこ.png", RED, [0, 0, 0, 0]),
        discPicture("わくわく.png", BLUE, [0, 0, 0, 0]),
      ]);
    await expect(page.getByText("2個のスタンプを追加しました！")).toBeVisible({ timeout: 40_000 });
    await expect(stampTiles(page)).toHaveCount(2);
    await expect(page.getByRole("button", { name: "にこにこ" })).toBeVisible();

    // A picture goes through the cut-out editor first.
    await page.getByRole("button", { name: "画像からつくる" }).click();
    await cutOut(page, discPicture("まる.png"));
    await page
      .getByRole("dialog", { name: "パックに追加" })
      .getByRole("button", { name: "パックに追加！" })
      .click();
    await expect(stampTiles(page)).toHaveCount(3, { timeout: 30_000 });

    // Rename one and make it the cover.
    await page.getByRole("button", { name: "わくわく" }).click();
    const editor = page.getByRole("dialog", { name: "スタンプを編集" });
    await editor.getByLabel("なまえ").fill("どきどき");
    await editor.getByRole("button", { name: "保存する" }).click();
    await expect(page.getByRole("button", { name: "どきどき" })).toBeVisible();
    await page.getByRole("button", { name: "どきどき" }).click();
    await editor.getByRole("button", { name: "カバーにする" }).click();
    await expect(page.getByText("カバーにしました")).toBeVisible();
    await expect(
      stampTiles(page)
        .filter({ has: page.getByRole("button", { name: "どきどき" }) })
        .getByLabel("カバー"),
    ).toBeVisible();

    await status.getByRole("button", { name: "審査に出す" }).click();
    await expect(page.getByText("審査に出しました！結果を待っててね")).toBeVisible();
    await expect(status.getByText("審査中")).toBeVisible();
    await expect(status.getByRole("button", { name: "審査を取り下げる" })).toBeVisible();
    // While it waits, nothing can be changed.
    await expect(page.getByRole("button", { name: "画像からつくる" })).toBeDisabled();

    const mine = await rpc<{
      packs: { id: string; shortId: string; name: string; status: string; stampCount: number }[];
    }>(request, `${STUDIO}/ListMyPacks`, {}, me);
    expect(mine.packs.map((pack) => [pack.name, pack.status, pack.stampCount])).toEqual([
      [name, "PACK_STATUS_IN_REVIEW", 3],
    ]);

    // Withdrawing makes it a draft again (and keeps the shared queue clean).
    await status.getByRole("button", { name: "審査を取り下げる" }).click();
    await expect(status.getByText("下書き")).toBeVisible();
    await open(page, "/stamps/studio");
    await expect(
      page.getByTestId("studio-pack").filter({ hasText: name }).getByText("下書き"),
    ).toBeVisible();
  });

  test("stamps of a pack are reordered by dragging, removed, and the pack deleted", async ({
    page,
    request,
    me,
  }) => {
    const pack = await draftPack(request, me, { stamps: 3 });
    const order = async () =>
      stampTiles(page)
        .getByRole("button")
        .evaluateAll((buttons) => buttons.map((button) => button.getAttribute("aria-label")));

    await open(page, `/stamps/studio/${pack.shortId}`);
    await expect.poll(order).toEqual([`${pack.name}1`, `${pack.name}2`, `${pack.name}3`]);

    // Drag the first stamp onto the third.
    const from = await page.getByRole("button", { name: `${pack.name}1` }).boundingBox();
    const to = await page.getByRole("button", { name: `${pack.name}3` }).boundingBox();
    if (!from || !to) throw new Error("stamps are not on screen");
    await page.mouse.move(from.x + from.width / 2, from.y + from.height / 2);
    await page.mouse.down();
    await page.mouse.move(from.x + from.width / 2 + 12, from.y + from.height / 2, { steps: 3 });
    await page.mouse.move(to.x + to.width / 2, to.y + to.height / 2, { steps: 10 });
    await page.mouse.up();
    await expect.poll(order).toEqual([`${pack.name}2`, `${pack.name}3`, `${pack.name}1`]);
    await expect
      .poll(async () =>
        (
          await rpc<PackResponse>(request, `${STORE}/GetPack`, { packId: pack.id }, me)
        ).pack.stamps?.map((item) => item.stampRef.name),
      )
      .toEqual([`${pack.name}2`, `${pack.name}3`, `${pack.name}1`]);

    // A drag is not a tap: no editor opened. A tap opens it; the stamp can be removed there.
    await expect(page.getByRole("dialog")).toHaveCount(0);
    await page.getByRole("button", { name: `${pack.name}3` }).click();
    await page
      .getByRole("dialog", { name: "スタンプを編集" })
      .getByRole("button", { name: "削除" })
      .click();
    await page
      .getByRole("dialog", { name: "このスタンプを削除する？" })
      .getByRole("button", { name: "削除" })
      .click();
    await expect(stampTiles(page)).toHaveCount(2);

    await page.getByRole("button", { name: "パックのメニュー" }).click();
    await page.getByRole("menuitem", { name: "パックを削除" }).click();
    await page
      .getByRole("dialog", { name: "パックを削除する？" })
      .getByRole("button", { name: "パックを削除" })
      .click();
    await expect(page).toHaveURL(/\/stamps\/studio$/);
    await expect(page.getByText("まだパックがないよ")).toBeVisible();
  });

  test("an animated Lottie file becomes a stamp that plays", async ({ page, request, me }) => {
    const pack = await draftPack(request, me, { stamps: 0 });
    await open(page, `/stamps/studio/${pack.shortId}`);
    await page
      .locator('input[type="file"][multiple]')
      .setInputFiles(pulsingLottie("どきどき.lottie"));
    await expect(page.getByText("1個のスタンプを追加しました！")).toBeVisible({ timeout: 40_000 });

    const stored = await rpc<{
      pack: {
        summary: { animated?: boolean };
        stamps: { stampRef: { format: string; animationUrl?: string } }[];
      };
    }>(request, `${STORE}/GetPack`, { packId: pack.id }, me);
    expect(stored.pack.stamps[0]?.stampRef.format).toBe("STAMP_FORMAT_LOTTIE");
    expect(stored.pack.summary.animated).toBe(true);

    // The tile draws the animation, and the picture changes from frame to frame.
    const canvas = stampTiles(page).locator("canvas");
    await expect(canvas).toHaveAttribute("data-ready", "");
    const redPixels = () =>
      canvas.evaluate((element) => {
        const target = element as HTMLCanvasElement;
        const data =
          target.getContext("2d")?.getImageData(0, 0, target.width, target.height).data ??
          new Uint8ClampedArray();
        let count = 0;
        for (let offset = 0; offset < data.length; offset += 4) {
          if (
            (data[offset] ?? 0) > 180 &&
            (data[offset + 1] ?? 255) < 90 &&
            (data[offset + 3] ?? 0) > 200
          )
            count++;
        }
        return count;
      });
    await expect.poll(redPixels).toBeGreaterThan(200);
    const sizes = new Set<number>();
    await expect(async () => {
      sizes.add(await redPixels());
      expect(sizes.size).toBeGreaterThan(2);
    }).toPass({ timeout: 10_000 });
  });
});

test.describe("review", () => {
  /** Finds a submission in the queue, loading further pages when other packs are waiting too. */
  async function queueItem(page: Page, name: string): Promise<Locator> {
    const item = page.getByRole("article", { name });
    await expect(async () => {
      if (!(await item.isVisible()))
        await page.getByTestId("review-item").last().scrollIntoViewIfNeeded();
      await expect(item).toBeVisible({ timeout: 2_000 });
    }).toPass({ timeout: 30_000 });
    return item;
  }

  test("a moderator approves a submitted pack and it is in the store", async ({
    page,
    request,
  }) => {
    const pack = await submittedPack(request, { stamps: 2 });
    try {
      await signInAs(page, await adminAccount(request));
      await open(page, "/admin/stamps");
      await expect(page.getByRole("heading", { name: "スタンプ審査" })).toBeVisible();

      const item = await queueItem(page, pack.name);
      await expect(item.getByText(`@${pack.creator.handle}`)).toBeVisible();
      await expect(stampTiles(item)).toHaveCount(2);
      // A stamp can be looked at closely before deciding.
      await item.getByRole("button", { name: `${pack.name}1` }).click();
      await expect(page.getByRole("dialog", { name: `${pack.name}1` })).toBeVisible();
      await page.keyboard.press("Escape");

      await item.getByRole("button", { name: "承認" }).click();
      await expect(page.getByText(`「${pack.name}」を承認しました`)).toBeVisible();
      await expect(page.getByRole("article", { name: pack.name })).toHaveCount(0);

      // Published: anyone can open it in the store and add it.
      await open(page, `/stamps/${pack.shortId}`);
      await expect(page.getByRole("heading", { name: pack.name, level: 2 })).toBeVisible();
      await expect(page.getByRole("button", { name: "無料で追加する" })).toBeVisible();
      await open(page, `/stamps/creator/${pack.creator.userId}`);
      await expect(page.getByTestId("pack-card").filter({ hasText: pack.name })).toBeVisible();

      // The moderator can put it on the featured shelf.
      await open(page, "/admin/stamps");
      await page.getByRole("tab", { name: "ストア管理" }).click();
      await page.getByRole("tab", { name: "新着" }).click();
      const shelf = page.getByTestId("shelf-pack").filter({ hasText: pack.name });
      await expect(shelf).toBeVisible();
      await shelf.getByRole("switch").click();
      await expect(page.getByText(`「${pack.name}」をおすすめにのせました`)).toBeVisible();
      await expect
        .poll(async () =>
          (
            await rpc<{ packs?: { id: string }[] }>(request, `${STORE}/ListStorePacks`, {
              section: "STORE_SECTION_FEATURED",
              page: { limit: 50 },
            })
          ).packs?.some((listed) => listed.id === pack.id),
        )
        .toBe(true);
      // Off again: the featured shelf is shared with every other test.
      await shelf.getByRole("switch").click();
      await expect(page.getByText(`「${pack.name}」をおすすめからはずしました`)).toBeVisible();
    } finally {
      await withdraw(request, pack);
    }
  });

  test("a declined pack comes back to its creator with the reason", async ({ page, request }) => {
    const pack = await submittedPack(request, { stamps: 1 });
    try {
      await signInAs(page, await adminAccount(request));
      await open(page, "/admin/stamps");
      const item = await queueItem(page, pack.name);
      await item.getByRole("button", { name: "却下" }).click();
      const dialog = page.getByRole("dialog", { name: `「${pack.name}」を却下する` });
      // A reason is required.
      await dialog.getByRole("button", { name: "却下" }).click();
      await expect(page.getByText("理由を入力してね")).toBeVisible();
      await dialog.getByLabel("却下の理由").fill("背景が切り抜かれていません");
      await dialog.getByRole("button", { name: "却下" }).click();
      await expect(page.getByText(`「${pack.name}」を却下しました`)).toBeVisible();
      await expect(page.getByRole("article", { name: pack.name })).toHaveCount(0);

      // The creator reads why in the studio, and can submit again.
      await signInAs(page, pack.creator);
      await open(page, `/stamps/studio/${pack.shortId}`);
      const status = page.getByTestId("pack-status");
      await expect(status.getByText("却下", { exact: true })).toBeVisible();
      await expect(status.getByTestId("review-note")).toContainText("背景が切り抜かれていません");
      await status.getByRole("button", { name: "もう一度審査に出す" }).click();
      await expect(status.getByText("審査中")).toBeVisible();

      // The earlier decision travels with the pack for the next moderator.
      const queue = await rpc<{
        items?: {
          pack: { summary: { id: string } };
          history?: { decision: string; note?: string }[];
        }[];
      }>(
        request,
        `${REVIEW}/ListReviewQueue`,
        { page: { limit: 30 } },
        await adminAccount(request),
      );
      const waiting = queue.items?.find((entry) => entry.pack.summary.id === pack.id);
      if (waiting)
        expect(waiting.history?.[0]).toMatchObject({
          decision: "REVIEW_DECISION_REJECTED",
          note: "背景が切り抜かれていません",
        });
    } finally {
      await withdraw(request, pack);
    }
  });

  test("a creator the moderator trusts publishes the next pack without review", async ({
    page,
    request,
  }) => {
    const pack = await submittedPack(request, { stamps: 1 });
    try {
      await signInAs(page, await adminAccount(request));
      await open(page, "/admin/stamps");
      const item = await queueItem(page, pack.name);
      await expect(item.getByText("このクリエイターを信頼する")).toBeVisible();
      await item.getByRole("switch").click();
      await expect(page.getByText("信頼クリエイターにしました")).toBeVisible();
      await item.getByRole("button", { name: "承認" }).click();
      await expect(page.getByText(`「${pack.name}」を承認しました`)).toBeVisible();

      // The studio says so, and a new pack goes live with one tap.
      const next = await draftPack(request, pack.creator, { stamps: 1 });
      await signInAs(page, pack.creator);
      await open(page, "/stamps/studio");
      await expect(page.getByText("信頼クリエイター", { exact: true })).toBeVisible();
      await page.getByTestId("studio-pack").filter({ hasText: next.name }).click();
      const status = page.getByTestId("pack-status");
      await status.getByRole("button", { name: "公開する" }).click();
      await expect(page.getByText("公開しました！🎉")).toBeVisible();
      await expect(status.getByText("公開中")).toBeVisible();
      await expect(
        page
          .getByRole("region", { name: "パックの成績" })
          .getByRole("slider", { name: /日ごとのグラフ/ }),
      ).toBeVisible();
    } finally {
      await withdraw(request, pack);
    }
  });

  test("the review page is closed to people who are not staff", async ({ page, me: _me }) => {
    await open(page, "/admin/stamps");
    await expect(page.getByText("ここはスタッフ専用だよ")).toBeVisible();
    await expect(page.getByRole("heading", { name: "スタンプ審査" })).toHaveCount(0);
  });
});

test("a creator is told a pack sold without being told who bought it", async ({
  page,
  request,
  me,
}) => {
  const pack = await draftPack(request, me, { price: 40, stamps: 1 });
  await rpc(request, `${STUDIO}/SubmitPack`, { packId: pack.id }, me);
  await rpc(request, `${REVIEW}/ApprovePack`, { packId: pack.id }, await adminAccount(request));
  const buyer = await createAccount(request, "kaitai");
  await grantCoins(request, buyer);
  await rpc(request, `${LIBRARY}/PurchasePack`, { packId: pack.id, expectedPrice: 40 }, buyer);

  // The API never hands out the buyer: no actor on the entry, only "someone".
  const sold = async () =>
    (
      await rpc<{ notifications?: { kind: string; actor?: unknown; actorUnnamed?: boolean }[] }>(
        request,
        "tokuly.notify.v1.InboxService/ListNotifications",
        {},
        me,
      )
    ).notifications?.find((entry) => entry.kind === "stamp.pack.sold");
  await expect.poll(sold, { timeout: 30_000 }).toMatchObject({ actorUnnamed: true });
  expect((await sold())?.actor).toBeUndefined();

  // The inbox shows a neutral face, and nothing of the buyer.
  await open(page, "/notifications");
  const entry = page.getByRole("link").filter({ hasText: "スタンプが売れたよ!" });
  await expect(entry.getByRole("img", { name: "だれか" })).toBeVisible();
  await expect(entry).not.toContainText(buyer.displayName);
  await expect(entry).not.toContainText(buyer.handle);
});
