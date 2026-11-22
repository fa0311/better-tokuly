import type { APIRequestContext } from "@playwright/test";
import { type Account, createAccount, expect, rpc, test } from "./support";

interface Point {
  lat: number;
  lng: number;
}

interface Place {
  id: string;
  name: string;
  center: Point;
  label?: string;
}

/** Where the browser says the device is (see playwright.config.ts): Shibuya crossing. */
const DEVICE: Point = { lat: 35.6595, lng: 139.7005 };

// Stays are reported somewhere of the test's own in Tokyo: the database is
// shared, and a stay next to someone else's public place would be a visit
// there instead of a spot worth suggesting.
const somewhereInTokyo = (): Point => ({
  lat: 35.66 + Math.random() * 0.08,
  lng: 139.6 + Math.random() * 0.15,
});

function east(from: Point, meters: number): Point {
  return {
    lat: from.lat,
    lng: from.lng + meters / (111_320 * Math.cos((from.lat * Math.PI) / 180)),
  };
}

function metersBetween(a: Point, b: Point): number {
  const north = (b.lat - a.lat) * 111_320;
  const eastward = (b.lng - a.lng) * 111_320 * Math.cos((a.lat * Math.PI) / 180);
  return Math.hypot(north, eastward);
}

const myPlaces = async (request: APIRequestContext, as: Account) =>
  (await rpc<{ places?: Place[] }>(request, "tokuly.geo.v1.PlaceService/ListMyPlaces", {}, as))
    .places ?? [];

const uniqueName = (base: string) => `${base}${Math.random().toString(36).slice(2, 8)}`;

// The map draws with software WebGL here: give it time before touching what is on it.
const MAP_TIMEOUT = 40_000;

test("a place is created by dragging a pin, renamed and deleted", async ({ page, request, me }) => {
  test.slow();
  await page.goto("/places");
  await page.getByRole("link", { name: "場所を追加" }).click();
  await expect(page).toHaveURL(/\/places\/new$/);

  // The pin starts where the device is; drag it somewhere else.
  const pin = page.getByTestId("place-pin");
  await expect(pin).toBeVisible({ timeout: MAP_TIMEOUT });
  await expect(page.getByTestId("place-area")).toContainText("渋谷区");
  const box = (await pin.boundingBox())!;
  const grab = { x: box.x + box.width / 2, y: box.y + box.height / 2 };
  await page.mouse.move(grab.x, grab.y);
  await page.mouse.down();
  await page.mouse.move(grab.x + 40, grab.y - 30, { steps: 5 });
  await page.mouse.move(grab.x + 90, grab.y - 70, { steps: 5 });
  await page.mouse.up();

  await page.getByRole("textbox", { name: "なまえ" }).fill("ひみつ基地");
  await page.getByRole("button", { name: "☕" }).click();
  await page.getByRole("button", { name: "この場所を登録する" }).click();

  await expect(page.getByText("場所を登録したよ!")).toBeVisible();
  await expect(page).toHaveURL(/\/places\/[0-9a-f-]{36}$/);
  await expect(page.getByRole("heading", { name: "ひみつ基地", level: 1 })).toBeVisible();
  const [created] = await myPlaces(request, me);
  expect(created?.name).toBe("ひみつ基地");
  // The place is where the pin was dropped, not where it started.
  expect(metersBetween(DEVICE, created!.center)).toBeGreaterThan(20);
  expect(metersBetween(DEVICE, created!.center)).toBeLessThan(2000);

  // Rename it.
  await page.getByRole("link", { name: "編集する" }).click();
  await expect(page).toHaveURL(/\/edit$/);
  await page.getByRole("textbox", { name: "なまえ" }).fill("ひみつ基地2号");
  await page.getByRole("button", { name: "保存する" }).click();
  await expect(page.getByRole("heading", { name: "ひみつ基地2号", level: 1 })).toBeVisible();
  expect((await myPlaces(request, me))[0]?.name).toBe("ひみつ基地2号");

  // Delete it, behind a confirmation.
  await page.getByRole("button", { name: "メニュー" }).click();
  await page.getByRole("menuitem", { name: "削除する" }).click();
  await page
    .getByRole("dialog", { name: "「ひみつ基地2号」を削除する?" })
    .getByRole("button", { name: "削除する" })
    .click();
  await expect(page).toHaveURL(/\/places$/);
  await expect(page.getByText("じぶんの場所を作ろう!")).toBeVisible();
  expect(await myPlaces(request, me)).toHaveLength(0);
});

test("a spot the server noticed is suggested, and accepting it makes it a place", async ({
  page,
  request,
  me,
}) => {
  // Three stays of six minutes at one spot within the last hour and a half, each ended by walking off.
  const spot = somewhereInTokyo();
  const samples = [];
  for (const minutesAgo of [90, 60, 30]) {
    const start = Date.now() - minutesAgo * 60_000;
    for (let index = 0; index <= 12; index++)
      samples.push({
        position: spot,
        accuracyMeters: 10,
        recordedAt: new Date(start + index * 30_000).toISOString(),
      });
    for (let index = 1; index <= 12; index++) {
      samples.push({
        position: east(spot, index * 25),
        accuracyMeters: 10,
        recordedAt: new Date(start + 6 * 60_000 + index * 10_000).toISOString(),
      });
    }
  }
  const reported = await rpc<{ acceptedCount?: number }>(
    request,
    "tokuly.geo.v1.LocationService/ReportLocations",
    { samples },
    me,
  );
  expect(reported.acceptedCount).toBe(samples.length);

  await page.goto("/places");
  const suggestions = page.getByRole("region", { name: "よく行く場所を登録しますか?" });
  await expect(suggestions.getByText(/ここ、「.+のよく行く場所」かも\?/)).toBeVisible();
  await expect(suggestions.getByText(/3回・合計 \d+分/)).toBeVisible();
  await suggestions.getByRole("button", { name: "登録する" }).click();

  await expect(page.getByText(/「.+のよく行く場所」を登録したよ!/)).toBeVisible();
  // It is a place of mine now, at that spot, and no longer a suggestion.
  await expect(
    page
      .getByRole("region", { name: "つくった場所" })
      .getByRole("link", { name: /のよく行く場所/ }),
  ).toBeVisible();
  await expect(suggestions).toHaveCount(0);
  const [place] = await myPlaces(request, me);
  expect(metersBetween(spot, place!.center)).toBeLessThan(60);
});

test("a suggestion can be put off", async ({ page, request, me }) => {
  const spot = somewhereInTokyo();
  const samples = [];
  for (const minutesAgo of [90, 60, 30]) {
    const start = Date.now() - minutesAgo * 60_000;
    for (let index = 0; index <= 12; index++)
      samples.push({
        position: spot,
        accuracyMeters: 10,
        recordedAt: new Date(start + index * 30_000).toISOString(),
      });
    for (let index = 1; index <= 12; index++) {
      samples.push({
        position: east(spot, index * 25),
        accuracyMeters: 10,
        recordedAt: new Date(start + 6 * 60_000 + index * 10_000).toISOString(),
      });
    }
  }
  await rpc(request, "tokuly.geo.v1.LocationService/ReportLocations", { samples }, me);

  await page.goto("/places");
  const suggestions = page.getByRole("region", { name: "よく行く場所を登録しますか?" });
  await suggestions.getByRole("button", { name: "あとで" }).click();
  await expect(suggestions).toHaveCount(0);
  await page.reload();
  await expect(page.getByRole("heading", { name: "いつもの場所" })).toBeVisible();
  await expect(suggestions).toHaveCount(0);
  expect(await myPlaces(request, me)).toHaveLength(0);

  // What the device remembers is this account's, holds no coordinates, and goes on sign-out.
  const remembered = () =>
    page.evaluate(() =>
      Object.entries(window.localStorage).filter(([key]) => key.startsWith("tokuly.places.")),
    );
  const entries = await remembered();
  expect(entries.map(([key]) => key)).toEqual([`tokuly.places.dismissed.${me.userId}`]);
  expect(entries[0]![1]).toMatch(/^\["[0-9a-z]{1,7}"\]$/);
  await page.goto("/me");
  await page.getByRole("button", { name: "ログアウト" }).click();
  await expect(page).toHaveURL(/\/$/);
  expect(await remembered()).toEqual([]);
});

test("the pin can be placed without a pointer: move the map, then put the pin at its centre", async ({
  page,
  request,
  me,
}) => {
  test.slow();
  await page.goto("/places/new");
  await expect(page.getByTestId("place-pin")).toBeVisible({ timeout: MAP_TIMEOUT });

  // The map pans with the arrow keys; the button brings the pin to wherever its centre is now.
  await page.locator(".maplibregl-canvas").focus();
  for (let press = 0; press < 4; press++) await page.keyboard.press("ArrowRight");
  await page.waitForTimeout(1500);
  await page.getByRole("button", { name: "マップの中心にピンを置く" }).focus();
  await page.keyboard.press("Enter");

  await page.getByRole("textbox", { name: "なまえ" }).fill("キーボードの場所");
  await page.getByRole("button", { name: "この場所を登録する" }).click();
  await expect(page).toHaveURL(/\/places\/[0-9a-f-]{36}$/);
  const [place] = await myPlaces(request, me);
  expect(metersBetween(DEVICE, place!.center)).toBeGreaterThan(20);
});

test("a place page opens from search, and checking in, saving and reacting work there", async ({
  page,
  request,
  me,
}) => {
  test.slow();
  // Someone else's public place.
  const owner = await createAccount(request, "Owner");
  const name = uniqueName("たこ焼きe2e");
  const { place } = await rpc<{ place: Place }>(
    request,
    "tokuly.geo.v1.PlaceService/CreatePlace",
    {
      name,
      emoji: "🐙",
      label: "PLACE_LABEL_CUSTOM",
      center: somewhereInTokyo(),
      radiusMeters: 60,
      visibility: "VISIBILITY_PUBLIC",
      category: "food",
    },
    owner,
  );

  await page.goto("/places");
  await page.getByRole("tab", { name: "さがす" }).click();
  await page.getByRole("searchbox", { name: "場所をさがす" }).fill(name);
  await page.getByRole("link", { name: new RegExp(name) }).click();

  await expect(page).toHaveURL(new RegExp(`/places/${place.id}$`));
  await expect(page.getByRole("heading", { name, level: 1 })).toBeVisible();
  await expect(page.getByTestId("place-address")).toContainText("東京都");
  // The credit the place-name data asks for is next to the address.
  await expect(page.getByText("Place names ©")).toBeVisible();
  await expect(page.getByText("ごはん")).toBeVisible();
  await expect(page.getByText("まだ行ってないよ。着いたらチェックインしてみよう!")).toBeVisible();

  // Check in with a few words.
  await page.getByRole("button", { name: "チェックイン" }).click();
  const dialog = page.getByRole("dialog", { name: "チェックイン" });
  await expect(dialog.getByText(name)).toBeVisible();
  await dialog.getByRole("textbox", { name: "ひとこと" }).fill("たこ焼きなう");
  await dialog.getByRole("button", { name: "チェックイン!" }).click();
  await expect(page.getByText("チェックインしたよ!")).toBeVisible();
  await expect(
    page.getByRole("region", { name: "じぶんの訪問" }).getByText("1回行ったよ"),
  ).toBeVisible();
  const { visits } = await rpc<{
    visits?: { checkIn?: boolean; message?: string; place?: { id: string } }[];
  }>(request, "tokuly.geo.v1.HistoryService/ListVisits", {}, me);
  expect(visits?.[0]).toMatchObject({
    checkIn: true,
    message: "たこ焼きなう",
    place: { id: place.id },
  });

  // Add it to my favourites: it shows up on that tab.
  await page.getByRole("button", { name: "お気に入りに追加" }).click();
  await expect(page.getByRole("button", { name: "お気に入り済み" })).toBeVisible();

  // A public place is a subject: it takes reactions and comments.
  const talk = page.getByRole("region", { name: "リアクションとコメント" });
  await expect(talk).toBeVisible();
  await talk.getByLabel("コメントを書く…").fill("ここのたこ焼きすき!");
  await talk.getByRole("button", { name: "コメント", exact: true }).click();
  await expect(talk.getByTestId("comment-text")).toHaveText("ここのたこ焼きすき!");

  await page.goto("/places?tab=saved");
  await expect(page.getByRole("link", { name: new RegExp(name) })).toBeVisible();
});

test("checking in here uses the current position and lands in today's history", async ({
  page,
  request,
  me,
}) => {
  await page.goto("/places");
  await page.getByRole("button", { name: "ここでチェックイン" }).click();
  const dialog = page.getByRole("dialog", { name: "チェックイン" });
  await expect(dialog.getByRole("button", { name: "いまここ(渋谷区)" })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  await dialog.getByRole("button", { name: "チェックイン!" }).click();
  await expect(page.getByText("チェックインしたよ!")).toBeVisible();

  const { visits } = await rpc<{ visits?: { checkIn?: boolean; center?: Point }[] }>(
    request,
    "tokuly.geo.v1.HistoryService/ListVisits",
    {},
    me,
  );
  expect(visits).toHaveLength(1);
  expect(visits![0]!.checkIn).toBe(true);
  expect(metersBetween(DEVICE, visits![0]!.center!)).toBeLessThan(100);

  // The toast leads to the day it belongs to.
  await page.getByRole("button", { name: "履歴を見る" }).click();
  await expect(page).toHaveURL(/\/history\/\d{4}-\d{2}-\d{2}$/);
  await expect(page.getByTestId("day-timeline").getByRole("listitem")).toHaveCount(1);
});

test("a place name from the map data can be turned into a place of mine", async ({
  page,
  me: _me,
}) => {
  await page.goto("/places?tab=search&q=渋谷");
  // Hits that are not Tokuly places (yet) offer to become one.
  const hit = page
    .getByRole("main")
    .getByRole("listitem")
    .filter({ has: page.getByRole("link", { name: "場所に登録" }) })
    .first();
  await expect(hit).toBeVisible();
  await expect(page.getByText("Place names ©")).toBeVisible();
  const name = (await hit.getByTestId("hit-name").textContent()) ?? "";
  expect(name).not.toBe("");
  await hit.getByRole("link", { name: "場所に登録" }).click();

  await expect(page).toHaveURL(/\/places\/new\?/);
  await expect(page.getByRole("textbox", { name: "なまえ" })).toHaveValue(name);
});

test("home is offered as one of the usual spots and stays private", async ({
  page,
  request,
  me,
}) => {
  await page.goto("/places");
  await page.getByRole("link", { name: "おうちを登録する" }).click();
  await expect(page).toHaveURL(/\/places\/new\?label=home$/);
  await expect(page.getByRole("button", { name: "おうち" })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  await expect(page.getByRole("textbox", { name: "なまえ" })).toHaveValue("おうち");
  await expect(
    page
      .getByText("おうち・学校・職場はいつも「じぶんだけ」。ともだちにはバッジだけ見えるよ")
      .first(),
  ).toBeVisible();
  await page.getByRole("button", { name: "この場所を登録する" }).click();

  await expect(page).toHaveURL(/\/places\/[0-9a-f-]{36}$/);
  const [home] = await rpc<{ places: { label: string; visibility: string }[] }>(
    request,
    "tokuly.geo.v1.PlaceService/ListMyPlaces",
    {},
    me,
  ).then((response) => response.places);
  expect(home).toMatchObject({ label: "PLACE_LABEL_HOME", visibility: "VISIBILITY_PRIVATE" });
});
