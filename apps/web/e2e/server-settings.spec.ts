import { readFile } from "node:fs/promises";
import type { Page } from "@playwright/test";
import { adminAccount, expect, signInAs, test } from "./support";

// 「サーバー設定」 in the staff console: every backend setting with where its
// value comes from. The deployment under test reads a tokuly.toml that sets
// [identity] registration_open (tools/init-env.sh writes it) and runs with
// TOKULY_RATE_LIMITS=false in its environment; everything else is default.

/** Shows the rows matching `text` (sections fold up unless searching). */
async function find(page: Page, text: string) {
  const search = page.getByRole("searchbox", { name: "キー・環境変数・説明でさがす" });
  await search.fill(text);
}

const row = (page: Page, name: string) => page.getByRole("listitem", { name, exact: true });

test("only admins see the server settings", async ({ page, me }) => {
  expect(me.roles).not.toContain("admin");
  await page.goto("/admin/settings");
  await expect(page.getByText("ここはスタッフ専用だよ")).toBeVisible();
});

test("an admin sees where each value comes from, changes one and exports the result", async ({
  page,
  request,
}, testInfo) => {
  // The settings are one per server: one browser size changes them.
  test.skip(testInfo.project.name !== "desktop", "the server settings are global");
  test.setTimeout(90_000);
  const admin = await adminAccount(request);
  await signInAs(page, admin);
  await page.goto("/admin/settings");
  await expect(page.getByRole("heading", { name: "サーバー設定" })).toBeVisible();
  await expect(page.getByText("読み込んだ設定ファイル:", { exact: false })).toBeVisible();

  // Not a console setting (set by the environment here), and a console
  // setting tokuly.toml sets: both locked, each with its reason.
  await find(page, "rate_limits");
  const rateLimits = row(page, "core.rate_limits");
  await expect(rateLimits.getByText("環境変数", { exact: true })).toBeVisible();
  await expect(rateLimits.getByText("tokuly.toml か環境変数で設定してね")).toBeVisible();
  await expect(rateLimits.getByRole("button", { name: /を変更$/ })).toHaveCount(0);
  await find(page, "registration_open");
  const registration = row(page, "identity.registration_open");
  await expect(registration.getByText("tokuly.toml", { exact: true })).toBeVisible();
  await expect(
    registration.getByText("tokuly.tomlで決まっているので、ここでは変えられないよ"),
  ).toBeVisible();
  await expect(registration.getByRole("button", { name: /を変更$/ })).toHaveCount(0);

  // Infrastructure, and a setting with a screen of its own.
  await find(page, "database_url");
  await expect(
    row(page, "core.database_url").getByText("tokuly.toml か環境変数で設定してね"),
  ).toBeVisible();
  await find(page, "openai_model");
  await expect(
    row(page, "chat.openai_model").getByRole("link", { name: "専用の画面をひらく" }),
  ).toHaveAttribute("href", "/admin/chat-ai");

  // A live setting: checked by the media domain's own rules, then applied at once.
  await find(page, "max_video_hours");
  const hours = row(page, "media.max_video_hours");
  await expect(hours.getByText("すぐ反映")).toBeVisible();
  await hours.getByRole("button", { name: "media.max_video_hours を変更" }).click();
  let dialog = page.getByRole("dialog");
  await dialog.getByLabel("新しい値").fill("0");
  await dialog.getByRole("button", { name: "保存" }).click();
  await expect(dialog.getByRole("alert")).toContainText("その値は使えないよ");
  await expect(dialog.getByRole("alert")).toContainText("at least 1");
  await dialog.getByLabel("新しい値").fill("4");
  await dialog.getByRole("button", { name: "保存" }).click();
  await expect(page.getByText("保存したよ！", { exact: true })).toBeVisible();
  await expect(hours.getByText("管理画面", { exact: true })).toBeVisible();
  await expect(hours.getByRole("definition").first()).toHaveText("4");

  // The app's identity at another service: a secret the console never changes.
  await find(page, "spotify_client_secret");
  const secret = row(page, "identity.spotify_client_secret");
  await expect(secret.getByText("tokuly.toml か環境変数で設定してね")).toBeVisible();
  await expect(secret.getByRole("button", { name: /を変更$/ })).toHaveCount(0);

  // A setting applied on restart, with its history.
  await find(page, "image_concurrency");
  const workers = row(page, "media.image_concurrency");
  await workers.getByRole("button", { name: "media.image_concurrency を変更" }).click();
  dialog = page.getByRole("dialog");
  await dialog.getByLabel("新しい値").fill("6");
  await dialog.getByRole("button", { name: "保存" }).click();
  await expect(page.getByText("保存したよ！再起動で反映されるよ")).toBeVisible();
  await expect(workers.getByText("再起動で反映されるよ（api / worker）")).toBeVisible();
  await workers.getByRole("button", { name: "media.image_concurrency の履歴" }).click();
  // Newest first; a server this test ran on before keeps the earlier rounds below.
  await expect(page.getByRole("dialog").getByRole("listitem").first()).toContainText("未設定 → 6");
  await page.keyboard.press("Escape");

  // The export: what is in force and not a default, without secrets.
  const [download] = await Promise.all([
    page.waitForEvent("download"),
    page.getByRole("button", { name: "設定を書き出す" }).click(),
  ]);
  expect(download.suggestedFilename()).toBe("tokuly.toml");
  const exported = await readFile(await download.path(), "utf8");
  expect(exported).toContain("[identity]\nregistration_open = true  # from tokuly.toml");
  expect(exported).toContain("max_video_hours = 4  # from 管理画面");
  expect(exported).toContain("rate_limits = false  # from TOKULY_RATE_LIMITS");
  expect(exported).toContain("image_concurrency = 6  # from 管理画面");

  // Both back to their defaults.
  for (const [query, name] of [
    ["max_video_hours", "media.max_video_hours"],
    ["image_concurrency", "media.image_concurrency"],
  ] as const) {
    await find(page, query);
    await row(page, name)
      .getByRole("button", { name: `${name} を変更` })
      .click();
    await page.getByRole("dialog").getByRole("button", { name: "既定値にもどす" }).click();
    await expect(page.getByText("もとにもどしたよ").first()).toBeVisible();
    await expect(row(page, name).getByText("既定値", { exact: true }).first()).toBeVisible();
  }
});

test("the server settings speak English too", async ({ browser, request }, testInfo) => {
  const admin = await adminAccount(request);
  const context = await browser.newContext({
    baseURL: testInfo.project.use.baseURL,
    locale: "en-US",
  });
  const page = await context.newPage();
  try {
    await signInAs(page, admin);
    await page.goto("/admin/settings?q=rate_limits");
    await expect(page.getByRole("heading", { name: "Server settings" })).toBeVisible();
    const rateLimits = row(page, "core.rate_limits");
    await expect(rateLimits.getByText("Environment", { exact: true })).toBeVisible();
    await expect(rateLimits.getByText("Set this in tokuly.toml or the environment")).toBeVisible();
    await expect(page.getByRole("button", { name: "Export settings" })).toBeVisible();
  } finally {
    await context.close();
  }
});
