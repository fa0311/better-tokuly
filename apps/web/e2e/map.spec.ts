import type { APIRequestContext, Browser, Locator, Page } from "@playwright/test";
import { type Account, befriend, createAccount, expect, PASSWORD, rpc, test } from "./support";

/** Where the browser is (the Playwright config's geolocation): Shibuya crossing. */
const HERE = { lat: 35.6595, lng: 139.7005 };

interface FriendLocation {
  userId: string;
  position?: { lat: number; lng: number };
  sharingPaused?: boolean;
  precision?: string;
}

/** A location sample as the API takes it, recorded `minutesAgo` minutes ago. */
function sample(lat: number, lng: number, minutesAgo = 0, extra: Record<string, unknown> = {}) {
  return {
    position: { lat, lng },
    accuracyMeters: 10,
    recordedAt: new Date(Date.now() - minutesAgo * 60_000).toISOString(),
    ...extra,
  };
}

/** Reports positions for an account through the API (what a phone in someone's pocket does). */
async function report(
  request: APIRequestContext,
  who: Account,
  ...samples: ReturnType<typeof sample>[]
) {
  await rpc(request, "tokuly.geo.v1.LocationService/ReportLocations", { samples }, who);
}

/** Where `viewer` sees `subject`, if at all. */
async function seenLocation(
  request: APIRequestContext,
  viewer: Account,
  subject: Account,
): Promise<FriendLocation | undefined> {
  const response = await rpc<{ locations?: FriendLocation[] }>(
    request,
    "tokuly.geo.v1.LocationService/ListFriendLocations",
    {},
    viewer,
  );
  return response.locations?.find((location) => location.userId === subject.userId);
}

/** Opens the map and waits until the browser's own position is on it (so it has been reported, too). */
async function openMap(page: Page, path = "/map") {
  await page.goto(path);
  await expect(page.getByTestId("own-pin")).toBeVisible({ timeout: 30_000 });
}

const pinOf = (page: Page, account: Account) =>
  page.locator(`[data-testid="friend-pin"][data-user-id="${account.userId}"]`);

/** Opens one of the panels behind the "more" button. */
async function openFromMore(page: Page, label: string) {
  await page.getByRole("button", { name: "もっと" }).click();
  await page.getByRole("menuitem", { name: label }).click();
}

/** Horizontal distance in pixels from my own pin to a friend's. */
async function offsetFromMe(page: Page, pin: Locator): Promise<number> {
  const [mine, theirs] = await Promise.all([
    page.getByTestId("own-pin").boundingBox(),
    pin.boundingBox(),
  ]);
  if (!mine || !theirs) return Number.NaN;
  return theirs.x + theirs.width / 2 - (mine.x + mine.width / 2);
}

/** A friend of `me` with a position `north` / `east` degrees away from the browser. */
async function friendAt(
  request: APIRequestContext,
  me: Account,
  name: string,
  north: number,
  east: number,
  extra: Record<string, unknown> = {},
): Promise<Account> {
  const friend = await createAccount(request, name);
  await befriend(request, me, friend);
  await report(request, friend, sample(HERE.lat + north, HERE.lng + east, 0, extra));
  return friend;
}

test("a friend's pin appears on the map and moves when they report a new position", async ({
  page,
  me,
  request,
}) => {
  const aoi = await friendAt(request, me, "Aoi", 0.001, 0.001);
  await openMap(page);
  const pin = pinOf(page, aoi);
  await expect(pin).toBeVisible();
  // The map settles on my own position first; measure against my pin so camera moves do not count.
  await expect.poll(() => offsetFromMe(page, pin)).toBeGreaterThan(5);
  const before = await offsetFromMe(page, pin);

  await report(request, aoi, sample(HERE.lat + 0.001, HERE.lng + 0.004));
  await expect
    .poll(() => offsetFromMe(page, pin), { timeout: 20_000 })
    .toBeGreaterThan(before + 40);
});

test("a pin shows battery and how long the friend has been there, and so does their sheet", async ({
  page,
  me,
  request,
}) => {
  const ren = await createAccount(request, "Ren");
  await befriend(request, me, ren);
  const spot = [HERE.lat - 0.002, HERE.lng + 0.002] as const;
  await report(
    request,
    ren,
    sample(...spot, 125, { batteryPercent: 60 }),
    sample(...spot, 60, { batteryPercent: 50 }),
    sample(...spot, 0, { batteryPercent: 42, charging: true }),
  );

  await openMap(page);
  const pin = pinOf(page, ren);
  await expect(pin.getByTestId("battery")).toContainText("42");
  await expect(pin.getByTestId("pin-caption")).toHaveText("2時間");

  await pin.click();
  const sheet = page.getByTestId("friend-sheet");
  await expect(sheet.getByRole("heading", { name: "Ren", exact: true })).toBeVisible();
  await expect(sheet.getByTestId("friend-facts")).toContainText("ここに2時間");
  await expect(sheet.getByTestId("friend-facts")).toContainText("充電中");
  // Walking distance and time, because we both see each other precisely.
  await expect(sheet.getByTestId("friend-where")).toContainText("m先");
  await expect(sheet.getByTestId("travel-time")).toContainText("分");
});

test("opening a friend's pin counts as a map view for them", async ({ page, me, request }) => {
  const aoi = await friendAt(request, me, "Aoi", 0.001, 0.001);
  await openMap(page, `/map?friend=${aoi.userId}`);
  await expect(
    page.getByTestId("friend-sheet").getByRole("heading", { name: "Aoi", exact: true }),
  ).toBeVisible();
  await expect
    .poll(
      async () =>
        (
          await rpc<{ viewerCount?: number }>(
            request,
            "tokuly.social.v1.InteractionService/GetMapViews",
            {},
            aoi,
          )
        ).viewerCount ?? 0,
    )
    .toBe(1);
});

test("a blurred friend is drawn as an area, without distance, route or trip", async ({
  page,
  me,
  request,
}) => {
  const mio = await friendAt(request, me, "Mio", 0.02, -0.02);
  await rpc(
    request,
    "tokuly.geo.v1.LocationService/SetGhostMode",
    { userIds: [me.userId], mode: "GHOST_MODE_BLURRED" },
    mio,
  );

  await openMap(page, `/map?friend=${mio.userId}`);
  const pin = pinOf(page, mio);
  await expect(pin).toHaveAttribute("data-state", "blurred");
  await expect(pin.getByTestId("pin-caption")).toHaveText("このへん");

  const sheet = page.getByTestId("friend-sheet");
  await expect(sheet.getByTestId("friend-where")).toContainText("おおよその場所だけ見えてるよ");
  await expect(sheet.getByTestId("friend-where")).not.toContainText("m先");
  await expect(sheet.getByTestId("travel-time")).toHaveCount(0);
  await expect(sheet.getByRole("button", { name: "向かう" })).toHaveAttribute(
    "aria-disabled",
    "true",
  );
});

test("freezing ghost mode stops my pin for the friend and stays visible on the map", async ({
  page,
  context,
  me,
  request,
}) => {
  const aoi = await friendAt(request, me, "Aoi", 0.001, 0.001);
  await openMap(page);
  const near = (target: { lat: number; lng: number }) => async () => {
    const position = (await seenLocation(request, aoi, me))?.position;
    return position ? Math.abs(position.lat - target.lat) + Math.abs(position.lng - target.lng) : 1;
  };
  await expect.poll(near(HERE), { timeout: 20_000 }).toBeLessThan(0.0005);

  await page.getByRole("button", { name: "ゴーストモード" }).click();
  await page.getByTestId("ghost-panel").getByTestId("ghost-frozen").click();
  const indicator = page.getByTestId("ghost-indicator");
  await expect(indicator).toContainText("フリーズ中");
  await expect(indicator).toContainText("みんなに");

  // I walk away: the friend keeps seeing the spot where I froze.
  const moved = { lat: HERE.lat + 0.01, lng: HERE.lng + 0.01 };
  await context.setGeolocation({ latitude: moved.lat, longitude: moved.lng, accuracy: 12 });
  await page.waitForTimeout(8000);
  expect(await near(HERE)()).toBeLessThan(0.0005);

  // Back to precise: the friend gets the live position, which proves it was being reported all along.
  await indicator.click();
  await page.getByTestId("ghost-panel").getByTestId("ghost-precise").click();
  await expect(indicator).toHaveCount(0);
  await expect.poll(near(moved), { timeout: 20_000 }).toBeLessThan(0.0005);
});

test("ghost mode can be set for one friend only", async ({ page, me, request }) => {
  const aoi = await friendAt(request, me, "Aoi", 0.001, 0.001);
  const ren = await friendAt(request, me, "Ren", -0.001, 0.002);
  await openMap(page);
  await expect
    .poll(async () => (await seenLocation(request, aoi, me))?.precision, { timeout: 20_000 })
    .toBe("GHOST_MODE_PRECISE");

  await page.getByRole("button", { name: "ゴーストモード" }).click();
  const row = page.getByTestId("ghost-friend-row").filter({ hasText: "Aoi" });
  await row.getByRole("radio", { name: "ぼかし" }).click();
  await expect(row).toContainText("この人だけ別の設定");
  await expect(page.getByTestId("ghost-indicator")).toContainText("1人に");

  await expect
    .poll(async () => (await seenLocation(request, aoi, me))?.precision)
    .toBe("GHOST_MODE_BLURRED");
  expect((await seenLocation(request, ren, me))?.precision).toBe("GHOST_MODE_PRECISE");
});

test("a map reaction from a friend plays on the map", async ({ page, me, request }) => {
  const aoi = await friendAt(request, me, "Aoi", 0.001, 0.001);
  await openMap(page);
  // The realtime connection may still be coming up: send until one arrives live.
  await expect(async () => {
    await rpc(
      request,
      "tokuly.social.v1.InteractionService/SendMapReaction",
      { userIds: [me.userId], kind: "MAP_REACTION_KIND_HEARTS", intensity: 3 },
      aoi,
    );
    await expect(page.getByTestId("effect-reaction").first()).toContainText("Aoiから", {
      timeout: 4000,
    });
  }).toPass({ timeout: 30_000 });
});

test("reactions received while away replay as a recap when the map opens", async ({
  page,
  me,
  request,
}) => {
  const aoi = await createAccount(request, "Aoi");
  await befriend(request, me, aoi);
  for (const kind of ["MAP_REACTION_KIND_HEARTS", "MAP_REACTION_KIND_EYES"]) {
    await rpc(
      request,
      "tokuly.social.v1.InteractionService/SendMapReaction",
      { userIds: [me.userId], kind, intensity: 1 },
      aoi,
    );
  }
  const pending = async () =>
    (
      await rpc<{ reactions?: unknown[] }>(
        request,
        "tokuly.social.v1.InteractionService/ListPendingMapReactions",
        {},
        me,
      )
    ).reactions?.length ?? 0;
  expect(await pending()).toBe(2);

  await openMap(page);
  await expect(page.getByTestId("effect-recap")).toContainText("るすの間にAoiから2件");
  // Played once: they are acknowledged and do not come back.
  await expect.poll(pending).toBe(0);
});

test("my reaction, a knock and the rules of buzz and bump from a friend's sheet", async ({
  page,
  me,
  request,
}) => {
  // About 5 km away: far too far for a buzz or a bump.
  const aoi = await friendAt(request, me, "Aoi", 0.03, 0.04);
  await openMap(page, `/map?friend=${aoi.userId}`);
  const sheet = page.getByTestId("friend-sheet");

  const buzz = sheet.getByTestId("nudge-buzz");
  await expect(buzz).toHaveAttribute("aria-disabled", "true");
  await expect(buzz).toContainText("200m以内にいるときだけ鳴らせるよ");
  await expect(buzz).toContainText(/いまは.+km先だよ/);
  const bump = sheet.getByTestId("nudge-bump");
  await expect(bump).toHaveAttribute("aria-disabled", "true");
  await expect(bump).toContainText("すぐそばでスマホを振り合うとバンプ");

  // Hearts fly to them and wait in their recap.
  await sheet.getByRole("button", { name: "ハート" }).click();
  await expect(page.getByText("飛ばしたよ！")).toBeVisible();
  await expect
    .poll(async () =>
      (
        await rpc<{ reactions?: { kind: string }[] }>(
          request,
          "tokuly.social.v1.InteractionService/ListPendingMapReactions",
          {},
          aoi,
        )
      ).reactions?.map((reaction) => reaction.kind),
    )
    .toEqual(["MAP_REACTION_KIND_HEARTS"]);

  // Three quick knocks go out as one burst.
  const knock = sheet.getByTestId("nudge-knock");
  await knock.click();
  await knock.click();
  await knock.click();
  await expect(page.getByText("Aoiを3回ノックしたよ！")).toBeVisible();
});

test("a buzz works for a friend who is right here", async ({ page, me, request }) => {
  const aoi = await friendAt(request, me, "Aoi", 0.0003, 0.0003);
  await openMap(page, `/map?friend=${aoi.userId}`);
  const buzz = page.getByTestId("friend-sheet").getByTestId("nudge-buzz");
  await expect(buzz).not.toHaveAttribute("aria-disabled", "true");
  // The server needs my own position, which the browser reports as the map opens.
  await expect(async () => {
    await buzz.click();
    await expect(page.getByText("Aoiにブザーを鳴らしたよ！")).toBeVisible({ timeout: 3000 });
  }).toPass({ timeout: 30_000 });
});

test("two friends in the same spot become a gathering marker with a member list", async ({
  page,
  me,
  request,
}) => {
  const hina = await createAccount(request, "Hina");
  const kai = await createAccount(request, "Kai");
  await befriend(request, me, hina);
  await befriend(request, me, kai);
  await befriend(request, hina, kai);
  // Side by side for four minutes (the server wants two).
  for (const minutesAgo of [4.5, 3.5, 2.5, 1.5, 0.5]) {
    await report(request, hina, sample(HERE.lat + 0.003, HERE.lng - 0.002, minutesAgo));
    await report(request, kai, sample(HERE.lat + 0.00303, HERE.lng - 0.00197, minutesAgo - 0.3));
  }

  await openMap(page);
  const marker = page.getByTestId("gathering-pin");
  await expect(marker).toContainText("2人が集合中!");
  await expect(pinOf(page, hina)).toHaveCount(0);

  await marker.click();
  const sheet = page.getByTestId("gathering-sheet");
  await expect(sheet.getByRole("heading", { name: "2人が集合中!" })).toBeVisible();
  await expect(sheet.getByText("Hina", { exact: true })).toBeVisible();
  await expect(sheet.getByText("Kai", { exact: true })).toBeVisible();
});

test("overlapping pins stack into one and fan out on tap", async ({ page, me, request }) => {
  // A little south-west of me and about 6 m apart: one stack at any zoom the map opens with.
  const aoi = await friendAt(request, me, "Aoi", -0.0015, -0.002);
  const ren = await friendAt(request, me, "Ren", -0.00145, -0.00197);
  await openMap(page);
  const stack = page.getByTestId("cluster-pin");
  await expect(stack).toHaveAccessibleName(/2人がこのあたり/);
  await expect(pinOf(page, aoi)).toHaveCount(0);

  await stack.click();
  await expect(pinOf(page, aoi)).toBeVisible();
  await expect(pinOf(page, ren)).toBeVisible();
  const [first, second] = await Promise.all([
    pinOf(page, aoi).boundingBox(),
    pinOf(page, ren).boundingBox(),
  ]);
  expect(
    Math.abs((first?.x ?? 0) - (second?.x ?? 0)) + Math.abs((first?.y ?? 0) - (second?.y ?? 0)),
  ).toBeGreaterThan(50);
});

test("the friend list sorts, searches and opens a friend", async ({ page, me, request }) => {
  await friendAt(request, me, "Aoi", 0.02, 0.02);
  await friendAt(request, me, "Ren", 0.001, 0.001);
  await openMap(page);
  await page.getByRole("button", { name: "ともだち", exact: true }).click();
  const list = page.getByTestId("friend-list");
  const rows = list.getByTestId("friend-row");
  await expect(rows).toHaveCount(2);
  // Nearest first.
  await expect(rows.first()).toContainText("Ren");

  await list.getByPlaceholder("なまえ または @ハンドル").fill("ao");
  await expect(rows).toHaveCount(1);
  await rows.first().click();
  await expect(
    page.getByTestId("friend-sheet").getByRole("heading", { name: "Aoi", exact: true }),
  ).toBeVisible();
});

test("a trip to a friend shows for both sides and can be ended", async ({ page, me, request }) => {
  const aoi = await friendAt(request, me, "Aoi", 0.004, 0.003);
  await openMap(page, `/map?friend=${aoi.userId}`);
  const tripsOf = async (who: Account) =>
    (
      await rpc<{ trips?: { user: { id: string } }[] }>(
        request,
        "tokuly.geo.v1.TripService/ListActiveTrips",
        {},
        who,
      )
    ).trips ?? [];

  // The server must know where I am before a trip can start.
  await expect
    .poll(async () => Boolean((await seenLocation(request, aoi, me))?.position), {
      timeout: 20_000,
    })
    .toBe(true);
  await page.getByTestId("friend-sheet").getByRole("button", { name: "向かう" }).click();
  await page.getByRole("button", { name: "出発！" }).click();

  const card = page.getByTestId("trips-panel").getByTestId("trip-card");
  await expect(card).toContainText("Aoiへ");
  await expect(card).toContainText("着）");
  await expect(page.getByTestId("trip-chip")).toContainText("Aoiへ");
  await expect(page.getByTestId("trip-destination")).toBeVisible();
  // The friend I am heading to sees it too.
  await expect
    .poll(async () => (await tripsOf(aoi)).map((trip) => trip.user.id))
    .toEqual([me.userId]);

  await card.getByRole("button", { name: "おわる" }).click();
  await page.getByRole("dialog").getByRole("button", { name: "おわる" }).click();
  await expect(card).toHaveCount(0);
  await expect.poll(async () => (await tripsOf(aoi)).length).toBe(0);
});

test("a place alert: I ask, the friend approves, it fires, I delete it", async ({
  page,
  me,
  request,
}) => {
  const aoi = await friendAt(request, me, "Aoi", 0.02, 0.02);
  await openMap(page, `/map?friend=${aoi.userId}`);
  await page.getByTestId("friend-sheet").getByRole("button", { name: "着いたら通知" }).click();

  const dialog = page.getByRole("dialog");
  await dialog.getByRole("combobox").click();
  await page.getByRole("option", { name: "じぶんがいまいる場所" }).click();
  await dialog.getByRole("switch").click();
  await dialog.getByRole("button", { name: "設定する" }).click();
  await expect(page.getByText("リクエストを送ったよ")).toBeVisible();

  await openFromMore(page, "みまもり");
  const row = page.getByTestId("safety-panel").getByTestId("alert-row");
  await expect(row).toContainText("Aoiが待ち合わせ場所に着いたら教えてもらう");
  await expect(row).toContainText("承認待ち");

  // The friend says yes; my list follows live.
  const alerts = await rpc<{ aboutMe: { id: string }[] }>(
    request,
    "tokuly.geo.v1.SafetyService/ListPlaceAlerts",
    {},
    aoi,
  );
  await rpc(
    request,
    "tokuly.geo.v1.SafetyService/RespondPlaceAlert",
    { alertId: alerts.aboutMe[0]?.id, approve: true },
    aoi,
  );
  await expect(row).toContainText("オン");

  // The friend is seen elsewhere, then arrives where I am.
  await report(request, aoi, sample(HERE.lat + 0.021, HERE.lng + 0.02));
  await page.waitForTimeout(1500);
  await report(request, aoi, sample(HERE.lat, HERE.lng));
  await expect(page.getByText("Aoiが待ち合わせ場所に着いたよ！")).toBeVisible({ timeout: 20_000 });

  await row.getByRole("button", { name: "削除する" }).click();
  await page.getByRole("dialog").getByRole("button", { name: "削除する" }).click();
  await expect(row).toHaveCount(0);
  await expect
    .poll(
      async () =>
        (
          await rpc<{ aboutMe?: unknown[] }>(
            request,
            "tokuly.geo.v1.SafetyService/ListPlaceAlerts",
            {},
            aoi,
          )
        ).aboutMe?.length ?? 0,
    )
    .toBe(0);
});

test("a friend's request for a recurring alert about me can be approved", async ({
  page,
  me,
  request,
}) => {
  const aoi = await friendAt(request, me, "Aoi", 0.02, 0.02);
  await rpc(
    request,
    "tokuly.geo.v1.SafetyService/CreatePlaceAlert",
    {
      subjectUserId: me.userId,
      center: HERE,
      placeName: "駅",
      trigger: "ALERT_TRIGGER_LEAVE",
      recurring: true,
    },
    aoi,
  );

  await openMap(page);
  await openFromMore(page, "みまもり");
  const row = page.getByTestId("safety-panel").getByTestId("alert-row");
  await expect(row).toContainText("あなたが駅を出たら、Aoiに通知がいく");
  await row.getByRole("button", { name: "OK" }).click();
  await expect(row).toContainText("オン");
  const mine = await rpc<{ mine: { status: string }[] }>(
    request,
    "tokuly.geo.v1.SafetyService/ListPlaceAlerts",
    {},
    aoi,
  );
  expect(mine.mine[0]?.status).toBe("ALERT_STATUS_ACTIVE");
});

test("SOS: hold, count down, friends get the live position, resolve", async ({
  page,
  me,
  request,
}) => {
  const aoi = await friendAt(request, me, "Aoi", 0.02, 0.02);
  // Frozen for everyone: an SOS must still show the friend where I am.
  await rpc(
    request,
    "tokuly.geo.v1.LocationService/SetGhostMode",
    { mode: "GHOST_MODE_FROZEN" },
    me,
  );
  await openMap(page);
  await openFromMore(page, "みまもり");

  const hold = page.getByTestId("sos-hold");
  await hold.dispatchEvent("pointerdown");
  await expect(page.getByTestId("sos-countdown")).toBeVisible({ timeout: 5000 });
  await hold.dispatchEvent("pointerup");
  // The countdown can be cut short — or cancelled.
  await page
    .getByRole("dialog")
    .getByPlaceholder("例：道にまよった、気分がわるい")
    .fill("道にまよった");
  await page.getByRole("dialog").getByRole("button", { name: "いますぐ送る" }).click();

  const banner = page.getByTestId("sos-mine");
  await expect(banner).toContainText("SOS発信中");
  const friendsSos = async () =>
    (
      await rpc<{
        friends?: { message: string; user: { id: string }; location?: { precision: string } }[];
      }>(request, "tokuly.geo.v1.SafetyService/GetActiveSos", {}, aoi)
    ).friends ?? [];
  await expect
    .poll(async () => (await friendsSos()).map((sos) => [sos.user.id, sos.message]))
    .toEqual([[me.userId, "道にまよった"]]);

  await banner.getByRole("button", { name: "もう大丈夫" }).click();
  await expect(banner).toHaveCount(0);
  await expect.poll(async () => (await friendsSos()).length).toBe(0);
});

test("a friend's SOS shows as a banner that leads to them", async ({ page, me, request }) => {
  const aoi = await friendAt(request, me, "Aoi", 0.003, 0.003);
  await openMap(page);
  await rpc(request, "tokuly.geo.v1.SafetyService/TriggerSos", { message: "たすけて" }, aoi);
  const banner = page.getByTestId("sos-friend");
  await expect(banner).toContainText("AoiがSOSを出してる！");
  await expect(banner).toContainText("たすけて");
  await banner.click();
  await expect(
    page.getByTestId("friend-sheet").getByRole("heading", { name: "Aoi", exact: true }),
  ).toBeVisible();
});

test("a live share link shows my moving pin to someone signed out, until it is revoked", async ({
  page,
  context,
  browser,
  me,
}) => {
  await openMap(page);
  await openFromMore(page, "リンクで共有");
  const panel = page.getByTestId("shares-panel");
  await panel.getByPlaceholder("例：駅までむかえに来て").fill("むかえに来て");
  await panel.getByRole("button", { name: "リンクをつくる" }).click();
  const row = panel.getByTestId("share-row");
  await expect(row).toContainText("むかえに来て");
  const url = (await row.getByTestId("share-url").textContent())?.trim() ?? "";
  expect(url).toMatch(/\/share\/\w{20,}$/);

  // Someone without an account opens the link.
  const outside = await browser.newContext({ locale: "ja-JP" });
  const visitor = await outside.newPage();
  await visitor.goto(url);
  await expect(visitor.getByTestId("share-card")).toContainText(`${me.displayName}のいま`);
  await expect(visitor.getByTestId("share-card")).toContainText("むかえに来て");
  const pin = visitor.getByTestId("share-pin");
  await expect(pin).toBeVisible({ timeout: 30_000 });
  await expect(pin).toHaveAttribute("data-lat", HERE.lat.toFixed(5));

  // I move; the visitor's pin follows.
  await context.setGeolocation({ latitude: HERE.lat + 0.002, longitude: HERE.lng, accuracy: 12 });
  await expect(pin).toHaveAttribute("data-lat", (HERE.lat + 0.002).toFixed(5), { timeout: 30_000 });

  await row.getByRole("button", { name: "共有をやめる" }).click();
  await page.getByRole("dialog").getByRole("button", { name: "共有をやめる" }).click();
  await expect(row).toHaveCount(0);
  await expect(visitor.getByTestId("share-ended")).toContainText("この共有はおわったよ", {
    timeout: 20_000,
  });

  // The link stays dead on a fresh visit.
  await visitor.reload();
  await expect(visitor.getByTestId("share-ended")).toBeVisible();
  await outside.close();
});

test("sharing can be stopped and resumed", async ({ page, me, request }) => {
  const aoi = await friendAt(request, me, "Aoi", 0.001, 0.001);
  await openMap(page);
  await expect
    .poll(async () => Boolean((await seenLocation(request, aoi, me))?.position), {
      timeout: 20_000,
    })
    .toBe(true);

  await page.getByTestId("own-pin").click();
  const sheet = page.getByTestId("me-sheet");
  await sheet.getByRole("button", { name: "共有をやめる" }).click();
  await expect(page.getByTestId("own-pin")).toHaveCount(0);
  await expect(page.getByTestId("sharing-notice")).toContainText("共有をお休み中");
  await expect.poll(async () => (await seenLocation(request, aoi, me))?.sharingPaused).toBe(true);

  await page.getByTestId("sharing-notice").getByRole("button", { name: "再開する" }).click();
  await expect(page.getByTestId("own-pin")).toBeVisible();
  await expect
    .poll(async () => (await seenLocation(request, aoi, me))?.sharingPaused ?? false, {
      timeout: 20_000,
    })
    .toBe(false);
});

/** A signed-in page in a browser context of its own (the `page` fixture always has location allowed). */
async function signedInPage(
  browser: Browser,
  baseURL: string | undefined,
  account: Account,
  permissions: string[],
) {
  const context = await browser.newContext({
    baseURL,
    locale: "ja-JP",
    permissions,
    geolocation: { latitude: HERE.lat, longitude: HERE.lng, accuracy: 12 },
  });
  const page = await context.newPage();
  const response = await page.request.post("/api/tokuly.identity.v1.AuthService/SignIn", {
    data: { identifier: account.handle, password: PASSWORD },
    headers: { "content-type": "application/json", "x-tokuly-client": "web" },
  });
  expect(response.ok()).toBeTruthy();
  await page.addInitScript(() => window.localStorage.setItem("tokuly.signedIn", "1"));
  return { context, page };
}

test("with location blocked the map says how to allow it, and starts once it is allowed", async ({
  browser,
  baseURL,
  request,
}) => {
  const { context, page } = await signedInPage(
    browser,
    baseURL,
    await createAccount(request, "Yui"),
    [],
  );
  await page.goto("/map");
  const notice = page.getByTestId("sharing-notice");
  await expect(notice).toContainText("位置情報がブロックされてるよ", { timeout: 30_000 });
  await expect(notice).toContainText("「位置情報」を許可して");
  await expect(page.getByTestId("own-pin")).toHaveCount(0);

  await context.grantPermissions(["geolocation"]);
  await page.reload();
  await expect(page.getByTestId("own-pin")).toBeVisible({ timeout: 30_000 });
  await expect(notice).toHaveCount(0);
  await context.close();
});

test("the browser is only asked for location when the person taps turn on", async ({
  browser,
  baseURL,
  request,
}) => {
  const { context, page } = await signedInPage(
    browser,
    baseURL,
    await createAccount(request, "Yui"),
    ["geolocation"],
  );
  // A browser that has not been asked yet, counting how often the page reaches for the position.
  await page.addInitScript(() => {
    const counter = window as unknown as { locationCalls: number };
    counter.locationCalls = 0;
    navigator.permissions.query = async () =>
      ({
        state: "prompt",
        addEventListener() {},
        removeEventListener() {},
      }) as unknown as PermissionStatus;
    const watch = navigator.geolocation.watchPosition.bind(navigator.geolocation);
    navigator.geolocation.watchPosition = (...args) => {
      counter.locationCalls += 1;
      return watch(...args);
    };
  });
  await page.goto("/map");
  const notice = page.getByTestId("sharing-notice");
  await expect(notice).toContainText("いまどこ？をシェアしよう！", { timeout: 30_000 });
  await expect(page.getByTestId("own-pin")).toHaveCount(0);
  expect(
    await page.evaluate(() => (window as unknown as { locationCalls: number }).locationCalls),
  ).toBe(0);

  await notice.getByRole("button", { name: "オンにする" }).click();
  await expect(page.getByTestId("own-pin")).toBeVisible({ timeout: 20_000 });
  await expect(notice).toHaveCount(0);
  await context.close();
});

test("map stories: the tray plays a friend's reel and remembers it was seen", async ({
  page,
  me,
  request,
}) => {
  const aoi = await friendAt(request, me, "Aoi", 0.003, 0.003);
  await rpc(
    request,
    "tokuly.geo.v1.PlaceService/CheckIn",
    { position: { lat: HERE.lat + 0.003, lng: HERE.lng + 0.003 }, message: "タピオカなう" },
    aoi,
  );
  const unseen = async () =>
    (
      await rpc<{ unseenReelCount?: number }>(
        request,
        "tokuly.feed.v1.MapStoryService/GetMapStoryTray",
        {},
        me,
      )
    ).unseenReelCount ?? 0;
  await expect.poll(unseen, { timeout: 20_000 }).toBe(1);

  await openMap(page);
  await page
    .getByTestId("story-tray")
    .getByRole("button", { name: /Aoiのストーリー/ })
    .click();
  const panel = page.getByTestId("story-panel");
  await expect(panel.getByTestId("story-item")).toContainText("チェックイン");
  await expect(page.getByTestId("story-pin")).toBeVisible();
  await expect.poll(unseen).toBe(0);
});

test("the own pin sits in the middle of the part of the map nothing covers", async ({
  page,
  me: _me,
}) => {
  await openMap(page);
  const viewport = page.viewportSize() ?? { width: 0, height: 0 };
  const phone = viewport.width < 768;
  // What covers the map: the card or side panel, and the tab bar on phones.
  const cover = await (phone
    ? page.getByTestId("map-empty")
    : page.getByTestId("friend-list")
  ).boundingBox();
  expect(cover).not.toBeNull();
  const free = phone
    ? { left: 0, right: viewport.width, top: 0, bottom: cover?.y ?? 0 }
    : {
        left: (cover?.x ?? 0) + (cover?.width ?? 0),
        right: viewport.width,
        top: 0,
        bottom: viewport.height,
      };

  await expect(async () => {
    const pin = await page.getByTestId("own-pin").boundingBox();
    expect(pin).not.toBeNull();
    const x = (pin?.x ?? 0) + (pin?.width ?? 0) / 2;
    const y = (pin?.y ?? 0) + (pin?.height ?? 0) / 2;
    // Within the middle third of the free area, both ways, and fully inside it.
    expect(Math.abs(x - (free.left + free.right) / 2)).toBeLessThan((free.right - free.left) / 6);
    expect(Math.abs(y - (free.top + free.bottom) / 2)).toBeLessThan((free.bottom - free.top) / 6);
    expect((pin?.y ?? 0) + (pin?.height ?? 0)).toBeLessThan(free.bottom);
  }).toPass({ timeout: 15_000 });

  if (phone) {
    // The empty-state card keeps its title on one line.
    const title = await page.getByTestId("map-empty").getByText("ともだちを呼ぼう！").boundingBox();
    expect(title?.height ?? 99).toBeLessThan(28);
  }
});

test("map controls work from the keyboard", async ({ page, me: _me }) => {
  await openMap(page);
  const toolbar = page.getByRole("toolbar", { name: "マップの操作" });
  await toolbar.getByRole("button", { name: "ゴーストモード" }).focus();
  await page.keyboard.press("Enter");
  await expect(page.getByTestId("ghost-panel")).toBeVisible();
  await page.getByTestId("ghost-panel").getByRole("button", { name: "閉じる" }).press("Enter");
  await expect(page.getByTestId("ghost-panel")).toHaveCount(0);
  await toolbar.getByRole("button", { name: "現在地へ" }).press("Enter");
  await expect(page.getByTestId("own-pin")).toBeVisible();
});
