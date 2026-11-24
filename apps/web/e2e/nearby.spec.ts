import type { APIRequestContext, Browser, Page } from "@playwright/test";
import { type Account, befriend, createAccount, expect, PASSWORD, rpc, test } from "./support";

/**
 * Nearby discovery and bumps are only taken where the account's own latest
 * fix puts it: the app sends a fresh fix first, and says so (with a retry)
 * when the server could not confirm the position.
 */

/** A spot of this test's own around Tokyo, so parallel tests do not meet each other. */
function somewhere(): { lat: number; lng: number } {
  return { lat: 35.55 + Math.random() * 0.25, lng: 139.55 + Math.random() * 0.25 };
}

/** A signed-in page in a browser context standing at `at`. */
async function pageAt(
  browser: Browser,
  baseURL: string | undefined,
  account: Account,
  at: { lat: number; lng: number },
): Promise<Page> {
  const context = await browser.newContext({
    baseURL,
    locale: "ja-JP",
    permissions: ["geolocation"],
    geolocation: { latitude: at.lat, longitude: at.lng, accuracy: 10 },
  });
  // Lets a test make the browser's one-off position reads fail (a device that has no fix).
  await context.addInitScript(() => {
    const flags = window as unknown as { failFix: boolean };
    flags.failFix = false;
    const read = navigator.geolocation.getCurrentPosition.bind(navigator.geolocation);
    navigator.geolocation.getCurrentPosition = (success, error, options) => {
      if (!flags.failFix) return read(success, error, options);
      error?.({
        code: 2,
        message: "no fix",
        PERMISSION_DENIED: 1,
        POSITION_UNAVAILABLE: 2,
        TIMEOUT: 3,
      });
    };
  });
  const page = await context.newPage();
  const response = await page.request.post("/api/tokuly.identity.v1.AuthService/SignIn", {
    data: { identifier: account.handle, password: PASSWORD },
    headers: { "content-type": "application/json", "x-tokuly-client": "web" },
  });
  expect(response.ok()).toBeTruthy();
  await page.addInitScript(() => window.localStorage.setItem("tokuly.signedIn", "1"));
  return page;
}

/** Turns sharing on (once per fresh account) and waits until the own position is known. */
async function shareHere(page: Page, path = "/map") {
  await page.goto(path);
  const pin = page.getByTestId("own-pin");
  const turnOn = page.getByTestId("sharing-notice").getByRole("button", { name: "オンにする" });
  await expect(pin.or(turnOn)).toBeVisible({ timeout: 30_000 });
  if (await turnOn.isVisible()) await turnOn.click();
  await expect(pin).toBeVisible({ timeout: 30_000 });
}

/** The friends screen's "find" tab, where nearby discovery is. */
async function findTab(page: Page) {
  await page.goto("/friends");
  await page.getByRole("tab", { name: "さがす" }).click();
}

async function openNearby(page: Page) {
  await findTab(page);
  const start = page.getByRole("button", { name: "近くの人をさがす" });
  await expect(start).toBeEnabled({ timeout: 30_000 });
  await start.click();
}

async function report(request: APIRequestContext, who: Account, sample: Record<string, unknown>) {
  await rpc(request, "tokuly.geo.v1.LocationService/ReportLocations", { samples: [sample] }, who);
}

test("two people with the add-friend screen open find each other", async ({
  browser,
  baseURL,
  request,
}) => {
  const spot = somewhere();
  const [aoi, ren] = [await createAccount(request, "Aoi"), await createAccount(request, "Ren")];
  const aoiPage = await pageAt(browser, baseURL, aoi, spot);
  const renPage = await pageAt(browser, baseURL, ren, { lat: spot.lat + 0.00005, lng: spot.lng });
  await Promise.all([shareHere(aoiPage), shareHere(renPage)]);
  await Promise.all([openNearby(aoiPage), openNearby(renPage)]);

  await expect(aoiPage.getByText("あなたから50m以内にいるユーザーを検索中です")).toBeVisible();
  await expect(aoiPage.getByText(ren.displayName, { exact: true }).first()).toBeVisible({
    timeout: 20_000,
  });
  await expect(renPage.getByText(aoi.displayName, { exact: true }).first()).toBeVisible({
    timeout: 20_000,
  });
  await expect(aoiPage.getByText("10m以内")).toBeVisible();
  await Promise.all([aoiPage.context().close(), renPage.context().close()]);
});

test("a position the server cannot confirm is explained, and retrying with a fresh fix works", async ({
  browser,
  baseURL,
  request,
}) => {
  const spot = somewhere();
  const yui = await createAccount(request, "Yui");
  const page = await pageAt(browser, baseURL, yui, spot);
  await shareHere(page);
  await findTab(page);
  await expect(page.getByRole("button", { name: "近くの人をさがす" })).toBeEnabled({
    timeout: 30_000,
  });

  // The device has no fix to give right now, and the last one the server got was a mock location.
  await page.evaluate(() => {
    (window as unknown as { failFix: boolean }).failFix = true;
  });
  await report(request, yui, {
    position: spot,
    accuracyMeters: 10,
    // (Two seconds ago: the server drops fixes less than a second apart, and the next one is near.)
    recordedAt: new Date(Date.now() - 2000).toISOString(),
    mocked: true,
  });
  await page.getByRole("button", { name: "近くの人をさがす" }).click();
  const notice = page.getByTestId("nearby-unverified");
  await expect(notice).toContainText("今いる場所を確認できなかったよ!位置情報をオンにしてね");

  // The device has a fix again (it moved a step): the retry sends it first and the search starts.
  await page.evaluate(() => {
    (window as unknown as { failFix: boolean }).failFix = false;
  });
  await page
    .context()
    .setGeolocation({ latitude: spot.lat + 0.00002, longitude: spot.lng, accuracy: 10 });
  await notice.getByRole("button", { name: "もう一度" }).click();
  await expect(page.getByText("あなたから50m以内にいるユーザーを検索中です")).toBeVisible();
  await expect(notice).toHaveCount(0);
  await page.context().close();
});

test("a bump from a friend's sheet sends a fresh fix and bumps a friend shaking at the same spot", async ({
  browser,
  baseURL,
  request,
}) => {
  const spot = somewhere();
  const [me, aoi] = [await createAccount(request, "Me"), await createAccount(request, "Aoi")];
  await befriend(request, me, aoi);
  await report(request, aoi, {
    position: spot,
    accuracyMeters: 10,
    recordedAt: new Date().toISOString(),
  });
  const page = await pageAt(browser, baseURL, me, spot);
  await shareHere(page, `/map?friend=${aoi.userId}`);

  await page.getByTestId("friend-sheet").getByTestId("nudge-bump").click();
  // Aoi shakes too, over the API, until the two shakes meet.
  await expect
    .poll(
      async () =>
        (
          await rpc<{ bumps?: unknown[] }>(
            request,
            "tokuly.social.v1.InteractionService/Bump",
            { location: spot, accuracyMeters: 10, intensity: 1.5 },
            aoi,
          )
        ).bumps?.length ?? 0,
      { intervals: [400] },
    )
    .toBe(1);
  await expect(page.getByText(`${aoi.displayName}とバンプ！`, { exact: false })).toBeVisible();
  await page.context().close();
});
