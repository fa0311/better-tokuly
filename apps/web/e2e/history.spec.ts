import type { APIRequestContext } from "@playwright/test";
import { type Account, expect, rpc, test } from "./support";

// ── A day's worth of history, through the API ───────────────────────────
// Visits come from stays: positions within 80 m of each other for five
// minutes. The server takes samples from the past, so a test reports a
// track that ended a moment ago: eight minutes in one place, a walk, eight
// minutes in another. Every test does that somewhere of its own in Tokyo
// (the database is shared, and a stay next to someone else's public place
// would be a visit there).

interface Point {
  lat: number;
  lng: number;
}

/**
 * The browser runs in a time zone where it is about noon right now, so the
 * track of the last half hour never straddles midnight and "today" is the
 * same day for the whole test.
 */
function middayZone(): string {
  let offset = 12 - new Date().getUTCHours();
  if (offset > 12) offset -= 24;
  // POSIX signs: "Etc/GMT-9" is nine hours ahead of UTC.
  return offset === 0 ? "Etc/GMT" : `Etc/GMT${offset > 0 ? "-" : "+"}${Math.abs(offset)}`;
}

const ZONE = middayZone();
test.use({ timezoneId: ZONE });

/** Today's date (`YYYY-MM-DD`) where the browser is. */
const today = () => new Intl.DateTimeFormat("sv-SE", { timeZone: ZONE }).format(new Date());

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

const sample = (position: Point, at: number, activity: string) => ({
  position,
  accuracyMeters: 10,
  speedMps: activity === "ACTIVITY_WALKING" ? 1.4 : 0,
  activity,
  recordedAt: new Date(at).toISOString(),
});

interface Day {
  first: Point;
  second: Point;
}

/**
 * Reports the last 28 minutes of `as`: eight minutes at `first`, a ten-minute
 * walk of 840 m east, eight minutes at `second`, then a few steps onwards
 * (which ends the second stay).
 */
async function liveADay(request: APIRequestContext, as: Account, first: Point): Promise<Day> {
  const second = east(first, 840);
  const start = Date.now() - 28 * 60_000;
  const samples = [];
  for (let index = 0; index <= 16; index++)
    samples.push(sample(first, start + index * 30_000, "ACTIVITY_STILL"));
  const walkStart = start + 8 * 60_000;
  for (let index = 1; index <= 60; index++)
    samples.push(sample(east(first, index * 14), walkStart + index * 10_000, "ACTIVITY_WALKING"));
  const arrived = walkStart + 10 * 60_000;
  for (let index = 1; index <= 16; index++)
    samples.push(sample(second, arrived + index * 30_000, "ACTIVITY_STILL"));
  const left = arrived + 8 * 60_000;
  for (let index = 1; index <= 9; index++)
    samples.push(sample(east(second, index * 14), left + index * 10_000, "ACTIVITY_WALKING"));
  const response = await rpc<{ acceptedCount?: number }>(
    request,
    "tokuly.geo.v1.LocationService/ReportLocations",
    { samples },
    as,
  );
  expect(response.acceptedCount, "every sample of the day was accepted").toBe(samples.length);
  return { first, second };
}

async function createPlace(
  request: APIRequestContext,
  as: Account,
  name: string,
  emoji: string,
  center: Point,
): Promise<string> {
  const response = await rpc<{ place: { id: string } }>(
    request,
    "tokuly.geo.v1.PlaceService/CreatePlace",
    {
      name,
      emoji,
      label: "PLACE_LABEL_CUSTOM",
      center,
      radiusMeters: 80,
      visibility: "VISIBILITY_PRIVATE",
    },
    as,
  );
  return response.place.id;
}

/** A day at two places of my own, so the visits have names to look for. */
async function dayAtCafeAndPark(
  request: APIRequestContext,
  as: Account,
): Promise<{ cafeId: string; parkId: string }> {
  const first = somewhereInTokyo();
  const cafeId = await createPlace(request, as, "あさのカフェ", "☕", first);
  const parkId = await createPlace(request, as, "ひるの公園", "🌳", east(first, 840));
  await liveADay(request, as, first);
  return { cafeId, parkId };
}

const listVisits = (request: APIRequestContext, as: Account) =>
  rpc<{ visits?: { id: string }[] }>(request, "tokuly.geo.v1.HistoryService/ListVisits", {}, as);

// The map draws with software WebGL here: give it time before looking at what is on it.
const MAP_TIMEOUT = 40_000;

test("a day with two visits shows both in order, the way between them and the route on the map", async ({
  page,
  request,
  me,
}) => {
  test.slow();
  await dayAtCafeAndPark(request, me);

  // The calendar marks today; its cell leads to the day.
  await page.goto("/history");
  const calendar = page.getByTestId("history-calendar");
  await expect(calendar.locator("[data-active]")).toHaveCount(1);
  await calendar.locator("[data-active]").click();
  await expect(page).toHaveURL(new RegExp(`/history/${today()}$`));

  // Both visits in the order of the day, with the walk between them.
  const items = page.getByTestId("day-timeline").getByRole("listitem");
  await expect(items).toHaveCount(3);
  await expect(items.nth(0)).toContainText("あさのカフェ");
  await expect(items.nth(0)).toContainText(/\d{1,2}:\d{2}–\d{1,2}:\d{2}・\d+分/);
  await expect(items.nth(1)).toContainText(/徒歩・\d+ m・\d+分/);
  await expect(items.nth(2)).toContainText("ひるの公園");
  await expect(page.getByRole("list", { name: "この日のまとめ" })).toContainText("2か所");

  // The route is on the map with its replay; the stops are pinned in order.
  const route = page.getByTestId("day-route");
  await expect(route).toBeVisible({ timeout: MAP_TIMEOUT });
  await expect(route).toHaveAttribute("data-timed", "true");
  expect(Number(await route.getAttribute("data-points"))).toBeGreaterThan(50);
  await expect(page.getByRole("button", { name: "1. あさのカフェ" })).toBeVisible();
  await expect(page.getByRole("button", { name: "2. ひるの公園" })).toBeVisible();

  // Scrubbing back to the start shows an earlier time than the end of the day.
  const end = await page.getByTestId("replay-time").textContent();
  await page.getByRole("slider", { name: "再生位置" }).focus();
  await page.keyboard.press("Home");
  await expect(page.getByTestId("replay-time")).not.toHaveText(end ?? "");
  await page.getByRole("button", { name: "ルートを再生" }).click();
  await expect(page.getByRole("button", { name: "一時停止" })).toBeVisible();
});

test("a memo is saved with the day and marks it on the calendar", async ({ page, request, me }) => {
  await dayAtCafeAndPark(request, me);
  await page.goto(`/history/${today()}`);

  const memo = page.getByRole("region", { name: "この日のメモ" });
  await memo.getByRole("textbox", { name: "この日のメモ" }).fill("カフェで新作を飲んだ!");
  await memo.getByRole("button", { name: "メモを保存" }).click();
  await expect(page.getByText("メモを保存したよ!")).toBeVisible();

  await page.reload();
  await expect(
    page
      .getByRole("region", { name: "この日のメモ" })
      .getByRole("textbox", { name: "この日のメモ" }),
  ).toHaveValue("カフェで新作を飲んだ!");

  await page.goto("/history");
  await expect(page.getByTestId("history-calendar").locator("[data-active]")).toContainText("📝");
});

test("deleting a day's history empties the day", async ({ page, request, me }) => {
  await dayAtCafeAndPark(request, me);
  await page.goto(`/history/${today()}`);
  await expect(page.getByTestId("day-timeline").getByRole("listitem")).toHaveCount(3);

  await page.getByRole("button", { name: "メニュー" }).click();
  await page.getByRole("menuitem", { name: "この日の履歴を削除" }).click();
  const confirm = page.getByRole("dialog", { name: "この日の履歴を削除する?" });
  await expect(confirm.getByText("削除すると元にもどせません")).toBeVisible();
  await confirm.getByRole("button", { name: "削除する" }).click();

  // The day is empty the moment the server confirmed, not after a re-read: no old visit, no old route.
  await expect(page.getByText("履歴を削除しました")).toBeVisible();
  await expect(page.getByTestId("day-timeline")).toHaveCount(0);
  await expect(page.getByTestId("day-route")).toHaveCount(0);
  await expect(page.getByText(/今日はまだどこにも行ってないみたい/)).toBeVisible();
  expect((await listVisits(request, me)).visits ?? []).toHaveLength(0);

  await page.goto("/history");
  await expect(page.getByTestId("history-calendar").locator("[data-active]")).toHaveCount(0);
});

test("everything can be deleted at once, footprints included", async ({ page, request, me }) => {
  await dayAtCafeAndPark(request, me);
  await page.goto("/history");
  await page.getByRole("button", { name: "メニュー" }).click();
  await page.getByRole("menuitem", { name: "履歴を削除…" }).click();

  const confirm = page.getByRole("dialog", { name: "履歴を削除する?" });
  await confirm.getByRole("tab", { name: "ぜんぶ" }).click();
  await confirm.getByRole("switch", { name: /あしあとも消す/ }).click();
  await confirm.getByRole("button", { name: "ぜんぶ削除する" }).click();
  await expect(page.getByText("履歴を削除しました")).toBeVisible();

  // Back to a blank diary, and the fog is back too.
  await expect(page.getByText("ここが、きみの日記になるよ")).toBeVisible();
  expect((await listVisits(request, me)).visits ?? []).toHaveLength(0);
  const footprints = await rpc<{ totalCellCount?: string }>(
    request,
    "tokuly.geo.v1.FootprintService/GetStats",
    {},
    me,
  );
  expect(Number(footprints.totalCellCount ?? 0)).toBe(0);
});

test("the list of visits can be narrowed to one place", async ({ page, request, me }) => {
  await dayAtCafeAndPark(request, me);
  await page.goto("/history?tab=visits");

  const visits = page.getByTestId("visit-list").getByRole("listitem");
  await expect(visits).toHaveCount(2);
  // Newest first.
  await expect(visits.nth(0)).toContainText("ひるの公園");
  await expect(visits.nth(1)).toContainText("あさのカフェ");

  await page.getByRole("combobox", { name: "場所でしぼる" }).click();
  await page.getByRole("option", { name: /あさのカフェ/ }).click();
  await expect(visits).toHaveCount(1);
  await expect(visits.nth(0)).toContainText("あさのカフェ");

  // A range that ended before today has nothing in it.
  await page.getByRole("tab", { name: "期間をえらぶ" }).click();
  const yesterday = new Intl.DateTimeFormat("sv-SE", { timeZone: ZONE }).format(
    new Date(Date.now() - 24 * 3_600_000),
  );
  await page.getByLabel("いつから").fill(yesterday);
  await page.getByLabel("いつまで").fill(yesterday);
  await expect(page.getByText("この条件のきろくはないよ")).toBeVisible();
});

test("the statistics add up the distance, the visits and the ways of getting around", async ({
  page,
  request,
  me,
}) => {
  await dayAtCafeAndPark(request, me);
  await page.goto("/history?tab=stats");

  await expect(page.getByTestId("stats-distance")).toHaveText(/\d+(\.\d+)? (m|km)/);
  const walking = page.getByRole("region", { name: "移動のしかた" });
  await expect(walking.getByText("徒歩")).toBeVisible();
  // The two places of the day are where this person keeps going.
  const top = page.getByRole("region", { name: "よく行く場所" });
  await expect(top.getByRole("link", { name: /あさのカフェ/ })).toBeVisible();
  await expect(top.getByRole("link", { name: /ひるの公園/ })).toBeVisible();

  await page.getByRole("tab", { name: "ぜんぶ" }).click();
  await expect(page.getByTestId("stats-distance")).toBeVisible();
});

test("a new account is told how the history fills up, on every tab", async ({ page, me: _me }) => {
  await page.goto("/history");
  await expect(page.getByText("ここが、きみの日記になるよ")).toBeVisible();
  await expect(page.getByRole("link", { name: "マップを開く" })).toHaveAttribute("href", "/map");

  await page.getByRole("tab", { name: "訪問" }).click();
  await expect(page.getByText("まだどこにも行ってないよ")).toBeVisible();
  await page.getByRole("tab", { name: "お泊まり" }).click();
  await expect(page.getByText("お泊まりのきろくはまだないよ")).toBeVisible();
  await page.getByRole("tab", { name: "きろく" }).click();
  await expect(page.getByText("この期間のきろくはまだないよ")).toBeVisible();

  // A day without anything still takes a memo.
  await page.goto(`/history/${today()}`);
  await expect(page.getByText(/今日はまだどこにも行ってないみたい/)).toBeVisible();
  await expect(page.getByRole("region", { name: "この日のメモ" })).toBeVisible();
});
