import type { APIRequestContext } from "@playwright/test";
import { type Account, befriend, createAccount, expect, rpc, test } from "./support";

// ── Walking, through the API ────────────────────────────────────────────
// History and footprints come from positions a device reports. The server
// takes samples from the past, so a test "walks" by reporting a track that
// ended a moment ago. Every test walks somewhere of its own in Tokyo: the
// database is shared, and a stay next to someone else's public place would
// count as a visit there.

interface Point {
  lat: number;
  lng: number;
}

const somewhereInTokyo = (): Point => ({
  lat: 35.66 + Math.random() * 0.08,
  lng: 139.6 + Math.random() * 0.15,
});

/** The point `meters` east of `from`. */
function east(from: Point, meters: number): Point {
  return {
    lat: from.lat,
    lng: from.lng + meters / (111_320 * Math.cos((from.lat * Math.PI) / 180)),
  };
}

/** A walk east from `from` at 1.4 m/s (a sample every 10 s) that ended `endedMinutesAgo` minutes ago. */
function walk(from: Point, meters: number, endedMinutesAgo: number) {
  const steps = Math.round(meters / 14);
  const start = Date.now() - endedMinutesAgo * 60_000 - steps * 10_000;
  return Array.from({ length: steps + 1 }, (_, index) => ({
    position: east(from, index * 14),
    accuracyMeters: 10,
    speedMps: 1.4,
    activity: "ACTIVITY_WALKING",
    recordedAt: new Date(start + index * 10_000).toISOString(),
  }));
}

async function report(request: APIRequestContext, as: Account, samples: object[]): Promise<void> {
  for (let index = 0; index < samples.length; index += 400) {
    const batch = samples.slice(index, index + 400);
    const response = await rpc<{ acceptedCount?: number }>(
      request,
      "tokuly.geo.v1.LocationService/ReportLocations",
      { samples: batch },
      as,
    );
    expect(response.acceptedCount, "every sample of the walk was accepted").toBe(batch.length);
  }
}

// The map draws with software WebGL here: give it time before looking at what is on it.
const MAP_TIMEOUT = 40_000;

test("a walk lifts the fog: cells on the map, a percentage and a level", async ({
  page,
  request,
  me,
}) => {
  test.slow();
  await report(request, me, walk(somewhereInTokyo(), 1200, 1));

  await page.goto("/footprints");
  // "東京都を 0.0…% 開拓した!"
  await expect(page.getByTestId("footprint-headline")).toHaveText(
    /東京都を \d+(\.\d+)?% 開拓した!/,
  );
  const hero = page.getByRole("region", { name: "開拓のきろく" });
  await expect(hero.getByText("おさんぽビギナー")).toBeVisible();
  await expect(hero.getByText(/あと \d+ マスで Lv\.\d+!/)).toBeVisible();

  // The prefecture and the country each have their bar.
  const regions = page.getByRole("region", { name: "エリアごとの開拓率" });
  await expect(regions.getByText("東京都")).toBeVisible();
  await expect(regions.getByRole("progressbar")).toBeVisible();
  await regions.getByRole("tab", { name: "国" }).click();
  await expect(regions.getByText("日本")).toBeVisible();

  // The map shows the uncovered cells around where the walk ended.
  const cells = page.getByTestId("footprint-cells");
  await expect(cells).toBeVisible({ timeout: MAP_TIMEOUT });
  await expect(cells).toHaveText(/このあたり \d+ マス/);
  expect(Number(await cells.getAttribute("data-cells"))).toBeGreaterThan(0);
});

test("the ranking lists a friend, and leads to their footprints", async ({ page, request, me }) => {
  test.slow();
  const friend = await createAccount(request, "Ren");
  await befriend(request, me, friend);
  await report(request, friend, walk(somewhereInTokyo(), 2000, 1));
  await report(request, me, walk(somewhereInTokyo(), 600, 1));

  await page.goto("/footprints");
  const board = page.getByRole("region", { name: "ランキング" });
  const rows = board.getByRole("listitem");
  await expect(rows).toHaveCount(2);
  // The friend walked further: first place for them, second for me.
  await expect(rows.nth(0)).toContainText("Ren");
  await expect(rows.nth(0)).toContainText("🥇");
  await expect(rows.nth(1)).toContainText("じぶん");

  // Everyone: only people who show their footprints publicly, so the friend is not there.
  await board.getByRole("tab", { name: "みんな" }).click();
  await expect(board.getByRole("link", { name: "Renのあしあとを見る" })).toHaveCount(0);
  await board.getByRole("tab", { name: "ともだち" }).click();

  await board.getByRole("link", { name: "Renのあしあとを見る" }).click();
  await expect(page).toHaveURL(new RegExp(`/footprints\\?user=${friend.userId}`));
  await expect(page.getByRole("heading", { name: "Renのあしあと" })).toBeVisible();
  await expect(page.getByText(/\d+ マス開拓中!/)).toBeVisible();
  await expect(page.getByTestId("footprint-cells")).toBeVisible({ timeout: MAP_TIMEOUT });
});

test("the recap shows what is new and starts over once it was seen", async ({
  page,
  request,
  me,
}) => {
  await report(request, me, walk(somewhereInTokyo(), 700, 1));

  await page.goto("/footprints");
  await page.getByRole("button", { name: /新しく \d+ マス開拓したよ!/ }).click();
  const recap = page.getByRole("dialog", { name: "あしあとレポート" });
  await expect(recap.getByText(/\+\d+ マス開拓!/)).toBeVisible();
  await expect(recap.getByText(/m 移動した/)).toBeVisible();
  await expect(recap.getByText("東京都に初上陸!")).toBeVisible();
  await expect(recap.getByRole("img", { name: "新しく開拓したマスのかたち" })).toBeVisible();
  await expect(recap.getByRole("button", { name: "画像でシェア" })).toBeVisible();

  // Closing it marks the recap as seen: the nudge goes away and the next recap starts empty.
  await recap.getByRole("button", { name: "close" }).click();
  await expect(page.getByRole("button", { name: /新しく \d+ マス開拓したよ!/ })).toHaveCount(0);
  await page.getByRole("button", { name: "レポート" }).click();
  await expect(
    page.getByRole("dialog", { name: "あしあとレポート" }).getByText("新しいマスはまだないよ"),
  ).toBeVisible();
});

test("a new account is told how footprints fill up", async ({ page, me: _me }) => {
  await page.goto("/footprints");
  await expect(page.getByText("まだ霧の中…!")).toBeVisible();
  await expect(page.getByRole("link", { name: "マップを開く" })).toHaveAttribute("href", "/map");
  // Alone on the board: an invitation instead of a ranking of one.
  await expect(page.getByText("ともだちを追加して、開拓バトルしよう!")).toBeVisible();
});

test("footprints someone keeps to themselves are simply not there", async ({
  page,
  request,
  me,
}) => {
  const friend = await createAccount(request, "Mio");
  await befriend(request, me, friend);
  await report(request, friend, walk(somewhereInTokyo(), 600, 1));
  // The settings group is replaced as a whole: read it, change the one field, write it back.
  const { settings } = await rpc<{ settings: { privacy: object } }>(
    request,
    "tokuly.identity.v1.AccountService/GetSettings",
    {},
    friend,
  );
  await rpc(
    request,
    "tokuly.identity.v1.AccountService/UpdateSettings",
    { privacy: { ...settings.privacy, footprintsVisibleTo: "AUDIENCE_NOBODY" } },
    friend,
  );

  await page.goto(`/footprints?user=${friend.userId}`);
  await expect(page.getByText("Mioのあしあとはひみつ")).toBeVisible();
  await expect(page.getByTestId("footprint-cells")).toHaveCount(0);
});

test("a failed request for someone's footprints offers another try, not a verdict", async ({
  page,
  request,
  me,
}) => {
  test.slow();
  const friend = await createAccount(request, "Sora");
  await befriend(request, me, friend);
  await report(request, friend, walk(somewhereInTokyo(), 600, 1));

  // The numbers cannot be fetched (once the page is retried, they can).
  let failing = true;
  await page.route("**/tokuly.geo.v1.FootprintService/GetStats", (route) =>
    failing ? route.abort() : route.continue(),
  );
  await page.goto(`/footprints?user=${friend.userId}`);
  await expect(page.getByText("読み込めませんでした")).toBeVisible();
  await expect(page.getByText("Soraのあしあとはひみつ")).toHaveCount(0);

  failing = false;
  await page.getByRole("button", { name: "もう一度" }).click();
  await expect(page.getByText(/\d+ マス開拓中!/)).toBeVisible();
  // Area names come with their credit.
  await expect(page.getByText("Place names ©")).toBeVisible();
});

test("celebrated levels are remembered per account, without naming regions, until sign-out", async ({
  page,
  request,
  me,
}) => {
  await report(request, me, walk(somewhereInTokyo(), 600, 1));
  await page.goto("/footprints");
  await expect(page.getByTestId("footprint-headline")).toBeVisible();

  const remembered = () =>
    page.evaluate(() =>
      Object.entries(window.localStorage).filter(([key]) => key.startsWith("tokuly.footprints.")),
    );
  await expect
    .poll(async () => (await remembered()).map(([key]) => key))
    .toEqual([`tokuly.footprints.levels.${me.userId}`]);
  const stored = (await remembered())[0]![1];
  expect(stored).not.toContain("JP");
  expect(Object.keys(JSON.parse(stored).regions).length).toBeGreaterThan(0);

  await page.goto("/me");
  await page.getByRole("button", { name: "ログアウト" }).click();
  await expect(page).toHaveURL(/\/$/);
  expect(await remembered()).toEqual([]);
});

test("the profile's explored share leads to the footprints", async ({ page, request, me }) => {
  await report(request, me, walk(somewhereInTokyo(), 600, 1));
  await page.goto(`/u/${me.handle}`);
  // The link says what it leads to and the number it shows.
  await page.getByRole("link", { name: /^あしあと: \d+(\.\d+)?% 開拓$/ }).click();
  await expect(page).toHaveURL(/\/footprints$/);
  await expect(page.getByTestId("footprint-headline")).toBeVisible();
});

test("the hub leads to places, history and footprints", async ({ page, me: _me }) => {
  await page.goto("/me");
  for (const [label, path] of [
    ["あしあと", "/footprints"],
    ["りれき", "/history"],
    ["場所", "/places"],
  ] as const) {
    await expect(
      page.getByRole("main").getByRole("link", { name: label, exact: true }),
    ).toHaveAttribute("href", path);
  }
});
