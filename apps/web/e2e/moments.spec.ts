import type { Page } from "@playwright/test";
import { postMoment, promptReached } from "./moments";
import { type Account, befriend, createAccount, expect, rpc, test } from "./support";

// Moments upload pairs of pictures and wait for the server to process them.
test.describe.configure({ timeout: 120_000 });

// Chrome's fake camera stands in for the device's: the browser takes the pictures itself.
test.use({
  permissions: ["geolocation", "notifications", "camera"],
  launchOptions: {
    args: [
      "--use-angle=swiftshader",
      "--enable-unsafe-swiftshader",
      "--ignore-gpu-blocklist",
      "--enable-webgl",
      "--disable-dev-shm-usage",
      "--use-fake-device-for-media-stream",
      "--use-fake-ui-for-media-stream",
    ],
  },
});

const MOMENTS = "tokuly.moments.v1.MomentService";

async function friendOf(request: Parameters<typeof befriend>[0], me: Account, name: string) {
  const friend = await createAccount(request, name);
  await befriend(request, me, friend);
  return friend;
}

const cardOf = (page: Page, name: string) =>
  page.getByTestId("moment-card").filter({ hasText: name });

/** Presses the shutter once the camera shows a picture. */
async function shoot(page: Page, label = "シャッター") {
  const shutter = page.getByRole("button", { name: label, exact: true });
  await expect(shutter).toBeEnabled({ timeout: 20_000 });
  await shutter.click();
}

test("the prompt counts down, the pair is taken in the browser, and friends' moments unlock", async ({
  page,
  me,
  request,
}) => {
  const aoi = await friendOf(request, me, "Aoi");
  promptReached([me, aoi]);
  await postMoment(request, aoi, { caption: "渋谷なう" });

  await page.goto("/moments");
  const day = page.getByTestId("moment-day");
  await expect(day).toHaveAttribute("data-phase", "open", { timeout: 30_000 });
  await expect(day.getByRole("timer")).toHaveText(/^[0-2]:\d\d$/);
  // Aoi's moment is there, blurred, and says nothing until mine is out.
  const card = cardOf(page, "Aoi");
  await expect(card.getByTestId("moment-pair")).toHaveAttribute("data-locked", "true");
  await expect(card.getByText("自分の瞬間をシェアするとみえるよ!")).toBeVisible();
  await expect(card.getByText("渋谷なう")).toHaveCount(0);

  await day.getByRole("link", { name: "撮る!" }).click();
  await expect(page).toHaveURL(/\/moments\/camera$/);
  await expect(page.getByTestId("moment-countdown")).toHaveText(/のこり [0-2]:\d\d/);
  // One camera: the back picture first, then the same camera turns to me.
  await shoot(page);
  await expect(page.getByText("つぎは内カメで自分!")).toBeVisible();
  await shoot(page);
  await page.getByRole("button", { name: "撮り直す" }).click();
  await shoot(page);
  await expect(page.getByText("つぎは内カメで自分!")).toBeVisible();
  await shoot(page);
  await page.getByRole("textbox", { name: "ひとこと(任意)" }).fill("はじめての瞬間");
  await page.getByRole("button", { name: "シェアする!" }).click();

  await expect(page).toHaveURL(/\/moments$/);
  await expect(day).toHaveAttribute("data-phase", "posted");
  await expect(day.getByText("時間内!")).toBeVisible();
  await expect(card.getByTestId("moment-pair")).not.toHaveAttribute("data-locked", "true");
  await expect(card.getByText("渋谷なう")).toBeVisible();
  await expect(card.getByRole("img", { name: "外カメの写真" })).toBeVisible();

  const today = await rpc<{ mine: { caption: string; retakes?: number } }>(
    request,
    `${MOMENTS}/GetToday`,
    {},
    me,
  );
  expect(today.mine).toMatchObject({ caption: "はじめての瞬間", retakes: 1 });
});

test("a moment after the window says how late it is", async ({ page, me, request }) => {
  const aoi = await friendOf(request, me, "Aoi");
  promptReached([me, aoi], 3 * 3600 + 300);
  await postMoment(request, aoi);

  await page.goto("/moments");
  const day = page.getByTestId("moment-day");
  await expect(day).toHaveAttribute("data-phase", "late", { timeout: 30_000 });
  await expect(day.getByRole("link", { name: "遅れて撮る" })).toBeVisible();

  await postMoment(request, me, { color: [40, 160, 220] });
  await page.reload();
  await expect(day).toHaveAttribute("data-phase", "posted");
  await expect(cardOf(page, "Aoi").getByTestId("moment-lateness")).toHaveText("3時間遅れ");
});

test("a RealMoji face is taken on the way, sent, kept in the library and taken back", async ({
  page,
  me,
  request,
}) => {
  const aoi = await friendOf(request, me, "Aoi");
  promptReached([me, aoi]);
  const moment = await postMoment(request, aoi);
  await postMoment(request, me, { color: [40, 160, 220] });

  await page.goto(`/moments/${moment}`);
  const detail = page.getByTestId("moment-detail");
  const smile = detail.getByRole("button", { name: "😃 にっこり" });
  await smile.click();
  const selfie = page.getByRole("dialog", { name: "RealMojiを撮る" });
  await expect(selfie.getByText("「😃 にっこり」の顔をしてパシャッ!")).toBeVisible();
  await shoot(page, "撮る");
  await expect(smile).toHaveAttribute("aria-pressed", "true", { timeout: 30_000 });
  await expect(detail.getByTestId("moment-reaction-list").getByText("me")).toBeVisible();
  await expect
    .poll(async () => {
      const { moment: seen } = await rpc<{ moment: { reactionCount?: number } }>(
        request,
        `${MOMENTS}/GetMoment`,
        { momentId: moment },
        aoi,
      );
      return seen.reactionCount ?? 0;
    })
    .toBe(1);

  // Tapping the face that is lit takes the reaction back.
  await smile.click();
  await expect(smile).toHaveAttribute("aria-pressed", "false");
  await expect(detail.getByText("リアクション 0")).toBeVisible();

  await page.goto("/moments");
  await page.getByRole("button", { name: "RealMoji" }).click();
  const library = page.getByTestId("realmoji-library");
  await expect(
    library
      .getByRole("listitem", { name: "😃 にっこり" })
      .getByRole("button", { name: "撮り直す" }),
  ).toBeVisible();
  await expect(
    library.getByRole("listitem", { name: "👍 いいね" }).getByRole("button", { name: "撮る" }),
  ).toBeVisible();
});

test("the calendar keeps my own moments, and a deleted one keeps the day used", async ({
  page,
  me,
  request,
}) => {
  promptReached([me]);
  const moment = await postMoment(request, me, { caption: "カレンダー用" });

  await page.goto("/moments/calendar");
  await expect(page.getByTestId("moment-calendar-summary")).toHaveText("1日シェア・時間内 1回");
  await page.getByTestId("moment-calendar-day").click();
  await expect(page).toHaveURL(new RegExp(`/moments/${moment}$`));
  await expect(page.getByText("カレンダー用")).toBeVisible();

  await page.goto("/moments");
  await page.getByRole("button", { name: "削除" }).click();
  await page.getByRole("dialog").getByRole("button", { name: "削除" }).click();
  await expect(page.getByText("今日の瞬間は削除したよ。また次の合図でね!")).toBeVisible();
  await page.goto("/moments/camera");
  await expect(page.getByText("今日の瞬間はもうシェアしたよ")).toBeVisible();
});
