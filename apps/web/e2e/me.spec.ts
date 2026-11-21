import { test as base } from "@playwright/test";
import { adminAccount, expect, signInAs, test } from "./support";

test("the hub shows who is signed in and leads to their profile", async ({ page, me }) => {
  await page.goto("/me");
  await expect(page.getByRole("heading", { name: "じぶん" })).toBeVisible();
  await expect(page.getByText(`@${me.handle}`)).toBeVisible();

  await page.getByRole("link", { name: /プロフィールを見る/ }).click();
  await expect(page).toHaveURL(new RegExp(`/u/${me.handle}$`));
});

test("the hub links to every personal area, and the admin console only for staff", async ({ page, me: _me }) => {
  await page.goto("/me");
  for (const [label, path] of [
    ["ともだち", "/friends"],
    ["お知らせ", "/notifications"],
    ["スタンプ", "/stamps"],
    ["ウォレット", "/wallet"],
    ["設定", "/settings"],
  ] as const) {
    await expect(page.getByRole("main").getByRole("link", { name: label })).toHaveAttribute("href", path);
  }
  await expect(page.getByRole("main").getByRole("link", { name: "管理" })).toHaveCount(0);
});

test("signing out returns to the landing page and locks the app again", async ({ page, me: _me }) => {
  await page.goto("/me");
  await page.getByRole("button", { name: "ログアウト" }).click();
  await expect(page).toHaveURL(/\/$/);

  await page.goto("/me");
  await expect(page).toHaveURL(/\/login/);
});

test("staff also get the admin console", async ({ page, request }) => {
  await signInAs(page, await adminAccount(request));
  await page.goto("/me");
  await expect(page.getByRole("main").getByRole("link", { name: "管理" })).toHaveAttribute("href", "/admin");
});

base("visitors without a session are sent to the login page", async ({ page }) => {
  await page.goto("/me");
  await expect(page).toHaveURL(/\/login\?redirect=/);
});
