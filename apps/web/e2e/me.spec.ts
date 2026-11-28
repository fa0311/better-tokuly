import { test as base } from "@playwright/test";
import {
  adminAccount,
  backOnline,
  createAccount,
  deployWhileOpen,
  expect,
  failToLoad,
  rpc,
  signInAs,
  test,
} from "./support";

test("the hub shows who is signed in and leads to their profile", async ({ page, me }) => {
  await page.goto("/me");
  await expect(page.getByRole("heading", { name: "じぶん" })).toBeVisible();
  await expect(page.getByText(`@${me.handle}`)).toBeVisible();

  await page.getByRole("link", { name: /プロフィールを見る/ }).click();
  await expect(page).toHaveURL(new RegExp(`/u/${me.handle}$`));
});

test("the hub links to every personal area, and the admin console only for staff", async ({
  page,
  me: _me,
}) => {
  await page.goto("/me");
  for (const [label, path] of [
    ["ともだち", "/friends"],
    ["お知らせ", "/notifications"],
    ["スタンプ", "/stamps"],
    ["ウォレット", "/wallet"],
    ["設定", "/settings"],
    ["アップデート情報", "/announcements"],
  ] as const) {
    await expect(page.getByRole("main").getByRole("link", { name: label })).toHaveAttribute(
      "href",
      path,
    );
  }
  await expect(page.getByRole("main").getByRole("link", { name: "管理" })).toHaveCount(0);
});

test("a badge whose count cannot load leaves the hub alone, and counts once the device is back online", async ({
  page,
  request,
  me,
}) => {
  const sender = await createAccount(request, "sender");
  await rpc(
    request,
    "tokuly.social.v1.FriendService/SendFriendRequest",
    { userId: me.userId },
    sender,
  );
  // The count's code fails to arrive, as when the connection drops right as the hub opens.
  const letThrough = await failToLoad(page, "badges.ts");

  await page.goto("/me");
  const friends = page.getByRole("main").getByRole("link", { name: /ともだち/ });
  await expect(friends).toHaveAccessibleName("ともだち");
  await expect(page.getByRole("heading", { name: "うまくいきませんでした" })).toHaveCount(0);

  await letThrough();
  await backOnline(page);
  await expect(friends).toHaveAccessibleName("1 ともだち");
});

test("after a deploy, a badge whose code is gone reloads the page once, into the new build", async ({
  page,
  request,
  me,
}) => {
  const sender = await createAccount(request, "sender");
  await rpc(
    request,
    "tokuly.social.v1.FriendService/SendFriendRequest",
    { userId: me.userId },
    sender,
  );
  // The count's code is not there, and neither is the build this page runs.
  const letThrough = await failToLoad(page, "badges.ts");
  const deploy = await deployWhileOpen(page, "reload");

  await page.goto("/me");
  await letThrough();
  await expect.poll(deploy.loads).toBe(2);
  await expect(page.getByRole("main").getByRole("link", { name: /ともだち/ })).toHaveAccessibleName(
    "1 ともだち",
  );
});

test("a build that stays gone after the reload is not reloaded again", async ({
  page,
  me: _me,
}) => {
  await failToLoad(page, "badges.ts");
  const deploy = await deployWhileOpen(page, "never");

  await page.goto("/me");
  await expect.poll(deploy.loads).toBe(2);
  await expect(page.getByRole("heading", { name: "じぶん" })).toBeVisible();
  // The badge is tried again when the device is back online: still gone, still no reload.
  await backOnline(page);
  await page.waitForTimeout(2_000);
  expect(deploy.loads()).toBe(2);
});

test("signing out returns to the landing page and locks the app again", async ({
  page,
  me: _me,
}) => {
  await page.goto("/me");
  await page.getByRole("button", { name: "ログアウト" }).click();
  await expect(page).toHaveURL(/\/$/);

  await page.goto("/me");
  await expect(page).toHaveURL(/\/login/);
});

test("staff also get the admin console", async ({ page, request }) => {
  await signInAs(page, await adminAccount(request));
  await page.goto("/me");
  await expect(page.getByRole("main").getByRole("link", { name: "管理" })).toHaveAttribute(
    "href",
    "/admin",
  );
});

base("visitors without a session are sent to the login page", async ({ page }) => {
  await page.goto("/me");
  await expect(page).toHaveURL(/\/login\?redirect=/);
});
