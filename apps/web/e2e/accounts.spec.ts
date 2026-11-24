import { expect as baseExpect, createAccount, PASSWORD, test } from "./support";

// Switching reloads the app as the other account; give it room on a busy machine.
test.describe.configure({ timeout: 120_000 });
const expect = baseExpect.configure({ timeout: 25_000 });

test("a second account is added beside the first and the two switch back and forth", async ({
  page,
  me,
  request,
}) => {
  const second = await createAccount(request, "ura");
  await page.goto("/settings/accounts");
  await expect(page.getByTestId("device-account")).toHaveCount(1);

  // Add the second one: a full sign-in to it, the first stays.
  await page.getByRole("link", { name: "アカウントを追加" }).click();
  await expect(page.getByRole("heading", { name: "アカウントを追加" })).toBeVisible();
  await page.getByLabel("メールアドレス または ハンドル").fill(second.handle);
  await page.getByLabel("パスワード").fill(PASSWORD);
  await page.getByRole("button", { name: "ログイン", exact: true }).click();
  await page.waitForURL("**/map");

  await page.goto("/settings/accounts");
  const rows = page.getByTestId("device-account");
  await expect(rows).toHaveCount(2);
  await expect(rows.filter({ hasText: "使用中" })).toContainText(`@${second.handle}`);

  // Back to the first, without a password.
  await rows
    .filter({ hasText: `@${me.handle}` })
    .getByRole("button", { name: "切り替え" })
    .click();
  await page.waitForURL("**/map");
  await page.goto("/settings/accounts");
  await expect(page.getByTestId("device-account").filter({ hasText: "使用中" })).toContainText(
    `@${me.handle}`,
  );

  // Signing the second out of this device leaves the first signed in.
  page.once("dialog", (dialog) => void dialog.accept());
  await page
    .getByTestId("device-account")
    .filter({ hasText: `@${second.handle}` })
    .getByRole("button", { name: /この端末からログアウト/ })
    .click();
  await expect(page.getByTestId("device-account")).toHaveCount(1);
  await expect(page.getByTestId("device-account")).toContainText(`@${me.handle}`);
});

test("signing out of one account offers the others on the sign-in screen", async ({
  page,
  me,
  request,
}) => {
  const second = await createAccount(request, "creator");
  // The login page is server-rendered: let it become interactive before typing.
  await page.goto("/login?add=true", { waitUntil: "networkidle" });
  await expect(page.getByRole("heading", { name: "アカウントを追加" })).toBeVisible();
  await page.getByLabel("メールアドレス または ハンドル").fill(second.handle);
  await page.getByLabel("パスワード").fill(PASSWORD);
  await page.getByRole("button", { name: "ログイン", exact: true }).click();
  await page.waitForURL("**/map");

  // Sign the active (second) account out from its profile.
  await page.goto(`/u/${second.handle}`);
  await page.getByRole("button", { name: "ログアウト", exact: true }).click();
  await page.goto("/login", { waitUntil: "networkidle" });
  const offered = page.getByRole("region", { name: "この端末のアカウントで続ける" });
  await expect(offered).toContainText(`@${me.handle}`);
  await expect(offered).not.toContainText(`@${second.handle}`);
  await offered.getByRole("button", { name: new RegExp(me.handle) }).click();
  await page.waitForURL("**/map");
  await page.goto("/settings/accounts");
  await expect(page.getByTestId("device-account").filter({ hasText: "使用中" })).toContainText(
    `@${me.handle}`,
  );
});

test("a switch in one tab reloads the other tab as the new account", async ({
  page,
  me,
  context,
  request,
}) => {
  const second = await createAccount(request, "tab");
  await page.goto("/login?add=true", { waitUntil: "networkidle" });
  await expect(page.getByRole("heading", { name: "アカウントを追加" })).toBeVisible();
  await page.getByLabel("メールアドレス または ハンドル").fill(second.handle);
  await page.getByLabel("パスワード").fill(PASSWORD);
  await page.getByRole("button", { name: "ログイン", exact: true }).click();
  await page.waitForURL("**/map");

  // A second tab of the same browser, on the accounts screen as the second account.
  const other = await context.newPage();
  await other.goto("/settings/accounts");
  const active = other.getByTestId("device-account").filter({ hasText: "使用中" });
  await expect(active).toContainText(`@${second.handle}`);

  // The first tab switches back to the main account.
  await page.goto("/settings/accounts");
  await page
    .getByTestId("device-account")
    .filter({ hasText: `@${me.handle}` })
    .getByRole("button", { name: "切り替え" })
    .click();
  await page.waitForURL("**/map");

  // The other tab starts over as the main account instead of acting as the old one.
  await expect(active).toContainText(`@${me.handle}`);
});

test("signing every account out of a shared computer leaves none on it", async ({
  page,
  me,
  request,
}) => {
  const second = await createAccount(request, "shared");
  await page.goto("/login?add=true", { waitUntil: "networkidle" });
  await expect(page.getByRole("heading", { name: "アカウントを追加" })).toBeVisible();
  await page.getByLabel("メールアドレス または ハンドル").fill(second.handle);
  await page.getByLabel("パスワード").fill(PASSWORD);
  await page.getByRole("button", { name: "ログイン", exact: true }).click();
  await page.waitForURL("**/map");

  await page.goto("/settings/data");
  await expect(page.getByText(/共用パソコン/)).toBeVisible();
  await page
    .getByRole("button", { name: "このブラウザのすべてのアカウントからログアウト" })
    .click();
  await page.waitForURL((url) => url.pathname === "/");
  await page.goto("/login", { waitUntil: "networkidle" });
  await expect(page.getByRole("heading", { name: "おかえりなさい！" })).toBeVisible();
  await expect(page.getByRole("region", { name: "この端末のアカウントで続ける" })).toHaveCount(0);
  expect(me.handle).toBeTruthy();
});
