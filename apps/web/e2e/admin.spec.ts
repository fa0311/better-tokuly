import { crc32, deflateSync } from "node:zlib";
import type { APIRequestContext, Page } from "@playwright/test";
import { type Account, adminAccount, createAccount, expect, PASSWORD, RpcError, refreshAccount, rpc, signInAs, test } from "./support";

const USERS = "tokuly.admin.v1.UserAdminService";
const MODERATION = "tokuly.admin.v1.ModerationService";
const FLAGS = "tokuly.admin.v1.FeatureFlagService";
const ANNOUNCEMENTS = "tokuly.admin.v1.AnnouncementService";

/** A short random tag that makes the things a test creates findable among everyone else's. */
const tag = () => `${Date.now().toString(36).slice(-5)}${Math.random().toString(36).slice(2, 6)}`;

/** A valid PNG of one colour, so image uploads need no fixture file. */
function solidPng(width: number, height: number, [red, green, blue]: [number, number, number]): Buffer {
  const chunk = (type: string, data: Buffer) => {
    const body = Buffer.concat([Buffer.from(type, "latin1"), data]);
    const length = Buffer.alloc(4);
    length.writeUInt32BE(data.length);
    const checksum = Buffer.alloc(4);
    checksum.writeUInt32BE(crc32(body));
    return Buffer.concat([length, body, checksum]);
  };
  const header = Buffer.alloc(13);
  header.writeUInt32BE(width, 0);
  header.writeUInt32BE(height, 4);
  header.set([8, 2, 0, 0, 0], 8); // 8-bit RGB, no interlacing
  const row = Buffer.concat([Buffer.from([0]), Buffer.alloc(width * 3, Buffer.from([red, green, blue]))]);
  const pixels = Buffer.concat(Array.from({ length: height }, () => row));
  return Buffer.concat([Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]), chunk("IHDR", header), chunk("IDAT", deflateSync(pixels)), chunk("IEND", Buffer.alloc(0))]);
}

/** Signs the browser in as the deployment's admin. */
async function asAdmin(page: Page, request: APIRequestContext): Promise<Account> {
  const admin = await adminAccount(request);
  await signInAs(page, admin);
  return admin;
}

/** The console's own navigation (a sidebar on wide screens, a scroller on phones). */
const consoleNav = (page: Page) => page.getByRole("navigation", { name: "管理コンソール" });

test.describe("who gets in", () => {
  test("an account without a staff role finds a friendly dead end, and the API refuses it too", async ({ page, request, me }) => {
    await page.goto("/admin");
    await expect(page.getByText("ここはスタッフ専用だよ")).toBeVisible();
    await expect(consoleNav(page)).toHaveCount(0);

    // A deep link fares no better.
    await page.goto("/admin/users");
    await expect(page.getByText("ここはスタッフ専用だよ")).toBeVisible();
    await page.getByRole("link", { name: "マップにもどる" }).click();
    await expect(page).toHaveURL(/\/map$/);

    await expect(rpc(request, `${USERS}/SearchUsers`, {}, me)).rejects.toMatchObject({ reason: "FORBIDDEN" });
  });

  test("a moderator gets moderation and users, but not the admin-only sections", async ({ page, request }) => {
    const admin = await adminAccount(request);
    const moderator = await createAccount(request, "mod");
    await rpc(request, `${USERS}/SetUserRoles`, { userId: moderator.userId, roles: ["moderator"], note: "e2e" }, admin);
    await signInAs(page, moderator);

    await page.goto("/admin");
    await expect(page.getByText("おつかれさま！")).toBeVisible();
    await expect(consoleNav(page).getByRole("link", { name: "モデレーション" })).toBeVisible();
    await expect(consoleNav(page).getByRole("link", { name: "ユーザー" })).toBeVisible();
    await expect(consoleNav(page).getByRole("link", { name: "機能フラグ" })).toHaveCount(0);
    await expect(consoleNav(page).getByRole("link", { name: "監査ログ" })).toHaveCount(0);

    await page.goto("/admin/flags");
    await expect(page.getByText("このページは管理者だけが開けるよ")).toBeVisible();

    // Roles are the admins' to change; sanctioning fellow staff is too.
    await page.goto(`/admin/users/${admin.userId}`);
    await expect(page.getByRole("button", { name: "警告を送る" })).toBeDisabled();
    await expect(page.getByRole("button", { name: "ロールを変更" })).toHaveCount(0);
    await expect(page.getByText("スタッフへの警告・停止・ログアウトは管理者だけができます。")).toBeVisible();
  });
});

test.describe("dashboard", () => {
  test("shows the deployment in numbers and refreshes on request", async ({ page, request }) => {
    await asAdmin(page, request);
    await page.goto("/admin");
    await expect(page.getByRole("heading", { name: "ダッシュボード" })).toBeVisible();

    // Accounts, activity, the moderation queue, content volume, queues and the backing services.
    await expect(page.getByText("アカウント数")).toBeVisible();
    await expect(page.getByRole("list", { name: "アクティブユーザー" }).getByRole("listitem")).toHaveCount(3);
    await expect(page.getByRole("region", { name: "モデレーション" }).getByText("対応待ち")).toBeVisible();
    await expect(page.getByRole("list", { name: "コンテンツ数" }).getByRole("listitem").first()).toBeVisible();
    await expect(page.getByRole("region", { name: "キューの状態" }).getByRole("heading", { name: "EVENTS" })).toBeVisible();
    await expect(page.getByRole("region", { name: "つながり先" }).getByText("postgres")).toBeVisible();

    const stats = page.waitForResponse((response) => response.url().endsWith("SystemService/GetServerStats") && response.ok());
    await page.getByRole("button", { name: "更新" }).click();
    await stats;
    await expect(page.getByText("アカウント数")).toBeVisible();
  });
});

test.describe("users", () => {
  test("searching an account, suspending it locks it out, lifting the suspension lets it back in", async ({ page, request, browser, baseURL }) => {
    await asAdmin(page, request);
    const target = await createAccount(request, "sus");

    await page.goto("/admin/users");
    await page.getByRole("searchbox", { name: "ユーザーをさがす" }).fill(target.handle);
    await page.getByRole("link", { name: new RegExp(`@${target.handle}`) }).click();
    await expect(page).toHaveURL(new RegExp(`/admin/users/${target.userId}$`));
    const identity = page.getByRole("region", { name: "ユーザー", exact: true });
    await expect(identity.getByText("有効", { exact: true })).toBeVisible();

    // Suspend for a day, with the reason the user will be told.
    await page.getByRole("button", { name: "アカウントを停止" }).click();
    const suspend = page.getByRole("dialog");
    await suspend.getByRole("button", { name: "停止する", exact: true }).click();
    await expect(suspend.getByText("理由を入力してね")).toBeVisible();
    await suspend.getByLabel("停止する期間").selectOption({ label: "1日" });
    await suspend.getByLabel("ユーザーに伝える理由").fill("e2e: スパム投稿のため");
    await suspend.getByRole("button", { name: "停止する", exact: true }).click();
    await expect(page.getByText("アカウントを停止しました")).toBeVisible();
    await expect(identity.getByText("停止中", { exact: true })).toBeVisible();
    await expect(page.getByRole("region", { name: "アカウント停止中" }).getByText("e2e: スパム投稿のため")).toBeVisible();

    // The suspended user is turned away at the login form.
    const visitorContext = await browser.newContext({ baseURL, locale: "ja-JP" });
    const visitor = await visitorContext.newPage();
    // The login page is server-rendered: let it become interactive before typing.
    await visitor.goto("/login", { waitUntil: "networkidle" });
    await visitor.locator('input[autocomplete~="username"]').fill(target.handle);
    await visitor.locator('input[type="password"]').fill(PASSWORD);
    await visitor.locator('form button[type="submit"]').click();
    await expect(visitor.getByRole("alert")).toBeVisible();
    await expect(visitor).toHaveURL(/\/login/);
    await visitorContext.close();
    await expect(rpc(request, "tokuly.identity.v1.AuthService/SignIn", { identifier: target.handle, password: PASSWORD })).rejects.toMatchObject({ reason: "ACCOUNT_BANNED" });

    // Lifting it shows at once, and the account can sign in again.
    await page.getByRole("region", { name: "アカウント停止中" }).getByRole("button", { name: "停止を解除" }).click();
    await page.getByRole("dialog").getByRole("button", { name: "解除する" }).click();
    await expect(page.getByText("停止を解除しました！")).toBeVisible();
    await expect(identity.getByText("有効", { exact: true })).toBeVisible();
    await expect(page.getByRole("region", { name: "アカウント停止中" })).toHaveCount(0);
    expect((await refreshAccount(request, target)).userId).toBe(target.userId);

    // Both actions are on the account's record.
    const recent = page.getByRole("region", { name: "最近の対応" });
    await expect(recent.getByText("一時停止", { exact: true })).toBeVisible();
    await expect(recent.getByText("停止を解除", { exact: true })).toBeVisible();
  });

  test("an admin grants a role and takes it away again", async ({ page, request }) => {
    await asAdmin(page, request);
    const target = await createAccount(request, "role");
    await page.goto(`/admin/users/${target.userId}`);
    const identity = page.getByRole("region", { name: "ユーザー", exact: true });
    await expect(identity.getByText(`@${target.handle}`)).toBeVisible();

    await page.getByRole("button", { name: "ロールを変更" }).click();
    const grant = page.getByRole("dialog");
    await grant.getByRole("switch", { name: /モデレーター/ }).click();
    await grant.getByRole("button", { name: "ロールを保存" }).click();
    await expect(page.getByText("ロールを変更しました")).toBeVisible();
    await expect(identity.getByText("モデレーター", { exact: true })).toBeVisible();
    expect((await refreshAccount(request, target)).roles).toEqual(["moderator"]);

    await page.getByRole("button", { name: "ロールを変更" }).click();
    const revoke = page.getByRole("dialog");
    await expect(revoke.getByRole("switch", { name: /モデレーター/ })).toBeChecked();
    await revoke.getByRole("switch", { name: /モデレーター/ }).click();
    await expect(revoke.getByText("ロールを外すと、相手はいったんログアウトされます。")).toBeVisible();
    await revoke.getByRole("button", { name: "ロールを保存" }).click();
    await expect(identity.getByText("モデレーター", { exact: true })).toHaveCount(0);
    expect((await refreshAccount(request, target)).roles).toEqual([]);
  });

  test("warning, the verified badge and signing out everywhere each ask first and show their result", async ({ page, request }) => {
    await asAdmin(page, request);
    const target = await createAccount(request, "acts");
    await page.goto(`/admin/users/${target.userId}`);
    const identity = page.getByRole("region", { name: "ユーザー", exact: true });

    await page.getByRole("button", { name: "警告を送る" }).click();
    const warn = page.getByRole("dialog");
    await warn.getByLabel("ユーザーに伝える理由").fill("e2e: 言葉づかいに気をつけてね");
    await warn.getByLabel("内部メモ").fill("e2e: 通報をもとに警告");
    await warn.getByRole("button", { name: "警告を送る" }).click();
    await expect(page.getByText("警告を送りました")).toBeVisible();
    await expect(page.getByRole("region", { name: "最近の対応" }).getByText("e2e: 言葉づかいに気をつけてね")).toBeVisible();

    await page.getByRole("button", { name: "認証バッジを付ける" }).click();
    await page.getByRole("dialog").getByRole("button", { name: "バッジを付ける" }).click();
    await expect(page.getByText("認証バッジを付けました！")).toBeVisible();
    await expect(identity.getByLabel("認証バッジ")).toBeVisible();
    await expect(page.getByRole("button", { name: "認証バッジを外す" })).toBeVisible();

    // The account signed up once, so one device is signed in.
    await page.getByRole("button", { name: "全端末からログアウトさせる" }).click();
    await page.getByRole("dialog").getByRole("button", { name: "ログアウトさせる" }).click();
    await expect(page.getByText(/\d+台をログアウトさせました/)).toBeVisible();
    await expect(rpc(request, "tokuly.identity.v1.AccountService/GetMe", {}, target)).rejects.toBeInstanceOf(RpcError);
  });
});

test.describe("feature flags", () => {
  /** On narrow screens the flag list and the per-user check are two tabs; wide screens show both. */
  const show = async (page: Page, tab: "フラグ一覧" | "ユーザーで確認") => {
    if ((page.viewportSize()?.width ?? 0) < 1024) await page.getByRole("tab", { name: tab }).click();
  };
  /** The row of one flag in the "evaluate for a user" result. */
  const evaluation = (page: Page, name: string) => page.getByRole("list", { name: "フラグの判定結果" }).getByRole("listitem").filter({ hasText: name });

  test("a flag is created, evaluated for a user and switched off", async ({ page, request }) => {
    const admin = await asAdmin(page, request);
    const subject = await createAccount(request, "flag");
    const name = `e2e_flag_${tag()}`;

    try {
      await page.goto("/admin/flags");
      await page.getByRole("button", { name: "新しいフラグ" }).click();
      const create = page.getByRole("dialog");
      await create.getByLabel("フラグ名").fill("Not a valid name");
      await create.getByRole("button", { name: "作成する" }).click();
      await expect(create.getByText(/フラグ名は英字で始めて/)).toBeVisible();
      await create.getByLabel("フラグ名").fill(name);
      await create.getByLabel("説明").fill("e2e: 作ってすぐ消すフラグ");
      await create.getByRole("switch", { name: /有効にする/ }).click();
      await create.getByRole("button", { name: "作成する" }).click();
      await expect(page.getByText("フラグを作成しました！")).toBeVisible();
      await page.getByRole("searchbox", { name: "フラグ名でさがす" }).fill(name);
      await expect(page.getByText(name, { exact: true })).toBeVisible();

      // Enabled at 100%: on for everyone.
      await show(page, "ユーザーで確認");
      await page.getByRole("combobox", { name: "確認するユーザー" }).fill(subject.handle);
      await page.getByRole("option", { name: new RegExp(`@${subject.handle}`) }).click();
      await expect(evaluation(page, name).getByText("全員に公開中なのでオン")).toBeVisible();

      // Switched off in the list, the same check turns to "off".
      await show(page, "フラグ一覧");
      await page.getByRole("switch", { name: `${name} を有効にする` }).click();
      await expect(page.getByText(`${name} を無効にしました`)).toBeVisible();
      await show(page, "ユーザーで確認");
      await expect(evaluation(page, name).getByText("フラグが無効なのでオフ")).toBeVisible();
    } finally {
      await rpc(request, `${FLAGS}/DeleteFlag`, { name }, admin).catch(() => {});
    }
  });

  test("a flag's allow-list is edited by user search, and the flag is deleted", async ({ page, request }) => {
    const admin = await asAdmin(page, request);
    const subject = await createAccount(request, "allow");
    const name = `e2e_flag_${tag()}`;
    await rpc(request, `${FLAGS}/CreateFlag`, { name, description: "e2e: 許可リストのテスト", enabled: false }, admin);

    try {
      // Opened from an account's page, the check starts with that account: the flag is off for it.
      await page.goto(`/admin/flags?q=${name}&evaluate=${subject.userId}`);
      await expect(evaluation(page, name).getByText("フラグが無効なのでオフ")).toBeVisible();

      // On the allow-list, the account gets the flag even though it is switched off.
      await show(page, "フラグ一覧");
      await page.getByRole("button", { name: `${name} を編集` }).click();
      const edit = page.getByRole("dialog");
      await edit.getByRole("combobox", { name: "ユーザーを追加" }).fill(subject.handle);
      await edit.getByRole("option", { name: new RegExp(`@${subject.handle}`) }).click();
      await edit.getByRole("spinbutton", { name: "公開率（%）" }).fill("30");
      await edit.getByRole("button", { name: "保存する" }).click();
      await expect(page.getByText("フラグを更新しました")).toBeVisible();
      await expect(page.getByText("30%")).toBeVisible();
      await show(page, "ユーザーで確認");
      await expect(evaluation(page, name).getByText("許可リストに入っているのでオン")).toBeVisible();

      await show(page, "フラグ一覧");
      await page.getByRole("button", { name: `${name} を削除` }).click();
      await page.getByRole("dialog").getByRole("button", { name: "削除する" }).click();
      await expect(page.getByText("フラグを削除しました")).toBeVisible();
      await expect(page.getByText(name, { exact: true })).toHaveCount(0);
      expect((await rpc<{ flags?: unknown[] }>(request, `${FLAGS}/ListFlags`, { query: name }, admin)).flags ?? []).toEqual([]);
    } finally {
      await rpc(request, `${FLAGS}/DeleteFlag`, { name }, admin).catch(() => {});
    }
  });
});

test.describe("announcements", () => {
  test("an announcement is written, published to the update log and taken back to draft", async ({ page, request }) => {
    const admin = await asAdmin(page, request);
    const title = `e2e お知らせ ${tag()}`;
    let id: string | undefined;

    try {
      await page.goto("/admin/announcements/new");

      // Japanese is required; English rides along.
      await page.getByRole("button", { name: "下書きを保存" }).click();
      const japanese = page.getByRole("region", { name: "日本語" });
      await expect(japanese.getByText("日本語のタイトルを1〜120文字で入力してね")).toBeVisible();
      await japanese.getByLabel("タイトル").fill(title);
      await japanese.getByLabel("本文").fill("## 変更点\n\n- **速く**なりました\n- [ヘルプ](https://tokuly.example/help)");
      await page.getByRole("region", { name: "English" }).getByLabel("タイトル").fill(`${title} (EN)`);

      // The preview shows the article as readers will see it.
      const preview = page.getByRole("region", { name: "プレビュー" });
      await expect(preview.getByRole("heading", { name: title }).first()).toBeVisible();
      await expect(preview.getByRole("heading", { name: "変更点" })).toBeVisible();

      await page.getByRole("button", { name: "下書きを保存" }).click();
      await expect(page.getByText("下書きを保存しました！")).toBeVisible();
      await expect(page).toHaveURL(/\/admin\/announcements\/[0-9a-f-]{36}$/);
      id = page.url().split("/").pop();
      const publishing = page.getByRole("region", { name: "公開", exact: true });
      await expect(publishing.getByText("下書き", { exact: true })).toBeVisible();

      // Publish now, without notifying anyone.
      await expect(publishing.getByRole("switch", { name: /みんなに通知する/ })).not.toBeChecked();
      await publishing.getByRole("button", { name: "今すぐ公開" }).click();
      await page.getByRole("dialog").getByRole("button", { name: "今すぐ公開" }).click();
      await expect(page.getByText("公開しました！")).toBeVisible();
      await expect(publishing.getByText("公開中", { exact: true })).toBeVisible();

      // Readers find it in the update log: the public page, then the list it sits in.
      await publishing.getByRole("link", { name: "公開ページを見る" }).click();
      await expect(page).toHaveURL((url) => url.pathname === `/announcements/${id}`);
      const article = page.getByRole("article");
      await expect(article.getByRole("heading", { level: 1, name: title })).toBeVisible();
      await expect(article.getByRole("link", { name: "ヘルプ" })).toHaveAttribute("href", "https://tokuly.example/help");
      await article.getByRole("link", { name: "アップデート情報にもどる" }).click();
      await expect(page).toHaveURL((url) => url.pathname === "/announcements");
      await expect(page.getByRole("link", { name: title, exact: true })).toBeVisible();

      // Unpublished, it is a draft again and gone from the update log.
      await page.goBack();
      await page.goBack();
      await expect(page).toHaveURL(new RegExp(`/admin/announcements/${id}$`));
      await publishing.getByRole("button", { name: "公開を取り下げる" }).click();
      await page.getByRole("dialog").getByRole("button", { name: "下書きにもどす" }).click();
      await expect(page.getByText("下書きにもどしました")).toBeVisible();
      await expect(publishing.getByText("下書き", { exact: true })).toBeVisible();
      await expect(publishing.getByRole("link", { name: "公開ページを見る" })).toHaveCount(0);
      const listed = await rpc<{ announcements?: { id: string }[] }>(request, `${ANNOUNCEMENTS}/ListAnnouncements`, { page: { limit: 50 } });
      expect((listed.announcements ?? []).map((announcement) => announcement.id)).not.toContain(id);
    } finally {
      if (id) await rpc(request, `${ANNOUNCEMENTS}/DeleteAnnouncement`, { id }, admin).catch(() => {});
    }
  });

  test("a header image is uploaded, previewed, saved with the announcement and removed again", async ({ page, request }) => {
    const admin = await asAdmin(page, request);
    const created = await rpc<{ announcement: { id: string } }>(request, `${ANNOUNCEMENTS}/CreateAnnouncement`, { title: { ja: `e2e 画像 ${tag()}` }, body: { ja: "ヘッダー画像つき" } }, admin);
    const id = created.announcement.id;
    const image = () => rpc<{ announcement: { image?: { url?: string } } }>(request, `${ANNOUNCEMENTS}/GetManagedAnnouncement`, { id }, admin).then((response) => response.announcement.image?.url);

    try {
      await page.goto(`/admin/announcements/${id}`);
      const preview = page.getByRole("region", { name: "プレビュー" });
      await expect(preview.getByRole("article").first()).toBeVisible();
      await expect(preview.getByRole("img")).toHaveCount(0);

      // The upload goes through the media service; the preview shows the image once it is processed.
      await page.getByLabel("画像を選ぶ").setInputFiles({ name: "header.png", mimeType: "image/png", buffer: solidPng(640, 280, [124, 92, 255]) });
      await expect(page.getByRole("button", { name: "画像を変える" })).toBeVisible({ timeout: 30_000 });
      await expect(preview.locator("img").first()).toBeVisible();
      await page.getByRole("button", { name: "保存する" }).click();
      await expect(page.getByText("保存しました", { exact: true })).toBeVisible();
      expect(await image()).toMatch(/^https?:\/\//);

      await page.getByRole("button", { name: "画像を外す" }).click();
      await expect(preview.locator("img")).toHaveCount(0);
      await page.getByRole("button", { name: "保存する" }).click();
      await expect(page.getByRole("button", { name: "保存する" })).toBeDisabled();
      await expect.poll(image).toBeUndefined();
    } finally {
      await rpc(request, `${ANNOUNCEMENTS}/DeleteAnnouncement`, { id }, admin).catch(() => {});
    }
  });

  test("a draft is scheduled, unscheduled and deleted", async ({ page, request }) => {
    const admin = await asAdmin(page, request);
    const title = `e2e 予約 ${tag()}`;
    const created = await rpc<{ announcement: { id: string } }>(request, `${ANNOUNCEMENTS}/CreateAnnouncement`, { title: { ja: title }, body: { ja: "あとで公開します" } }, admin);
    const id = created.announcement.id;

    try {
      // Found among the drafts of the staff list.
      await page.goto("/admin/announcements?status=draft");
      await page.getByRole("link", { name: title, exact: true }).click();
      await expect(page).toHaveURL(new RegExp(`/admin/announcements/${id}$`));
      const publishing = page.getByRole("region", { name: "公開", exact: true });
      await publishing.getByLabel("公開する日時").fill("2099-01-02T09:30");
      await publishing.getByRole("button", { name: "この日時に予約" }).click();
      const confirm = page.getByRole("dialog");
      await expect(confirm.getByText(/2099\/01\/02 09:30 に公開されます/)).toBeVisible();
      await confirm.getByRole("button", { name: "この日時に予約" }).click();
      // More than a year ahead is refused by the server, and said so.
      await expect(page.getByText("公開日時は1年以内にしてね")).toBeVisible();
      await confirm.getByRole("button", { name: "キャンセル" }).click();

      const soon = new Date(Date.now() + 3 * 86_400_000);
      const local = `${soon.getFullYear()}-${String(soon.getMonth() + 1).padStart(2, "0")}-${String(soon.getDate()).padStart(2, "0")}T09:30`;
      await publishing.getByLabel("公開する日時").fill(local);
      await publishing.getByRole("button", { name: "この日時に予約" }).click();
      await page.getByRole("dialog").getByRole("button", { name: "この日時に予約" }).click();
      await expect(page.getByText("公開を予約しました")).toBeVisible();
      await expect(publishing.getByText("予約済み", { exact: true })).toBeVisible();
      await expect(publishing.getByText("公開予定")).toBeVisible();

      // Not visible to readers until then.
      const listed = await rpc<{ announcements?: { id: string }[] }>(request, `${ANNOUNCEMENTS}/ListAnnouncements`, { page: { limit: 50 } });
      expect((listed.announcements ?? []).map((announcement) => announcement.id)).not.toContain(id);

      await publishing.getByRole("button", { name: "予約を取り消す" }).click();
      await page.getByRole("dialog").getByRole("button", { name: "下書きにもどす" }).click();
      await expect(publishing.getByText("下書き", { exact: true })).toBeVisible();

      // Deleting removes it for good and returns to the list.
      await publishing.getByRole("button", { name: "削除する" }).click();
      await page.getByRole("dialog").getByRole("button", { name: "削除する" }).click();
      await expect(page.getByText("お知らせを削除しました")).toBeVisible();
      await expect(page).toHaveURL(/\/admin\/announcements$/);
      await expect(rpc(request, `${ANNOUNCEMENTS}/GetManagedAnnouncement`, { id }, admin)).rejects.toMatchObject({ reason: "ANNOUNCEMENT_NOT_FOUND" });
    } finally {
      await rpc(request, `${ANNOUNCEMENTS}/DeleteAnnouncement`, { id }, admin).catch(() => {});
    }
  });
});

test.describe("moderation", () => {
  test("a queue with nothing waiting says so, with its filters in the address", async ({ page, request }) => {
    await asAdmin(page, request);
    const quiet = await createAccount(request, "quiet");

    // From an account's page: the cases about that account's content. A fresh account has none.
    await page.goto(`/admin/users/${quiet.userId}`);
    await page.getByRole("link", { name: "このユーザーのケース" }).click();
    await expect(page).toHaveURL(new RegExp(`/admin/moderation\\?owner=${quiet.userId}`));
    await expect(page.getByText("対応待ちのケースはありません")).toBeVisible();

    await page.getByRole("tab", { name: "解決済み" }).click();
    await expect(page).toHaveURL(/status=resolved/);
    await expect(page.getByText("解決済みのケースはまだありません")).toBeVisible();

    await page.getByLabel("通報理由").selectOption({ label: "スパム" });
    await page.getByLabel("担当", { exact: true }).selectOption({ label: "自分が担当" });
    await expect(page).toHaveURL(/category=1/);
    await expect(page).toHaveURL(/assignee=mine/);
    await expect(page.getByText("解決済みのケースはまだありません")).toBeVisible();
  });

  test("content is taken down by reference and the action lands in the moderation log", async ({ page, request }) => {
    const admin = await asAdmin(page, request);
    const suffix = tag();
    const reason = `e2e: 規約違反のため ${suffix}`;
    const note = `e2e: 直接削除のテスト ${suffix}`;
    const subjectId = crypto.randomUUID();

    await page.goto("/admin/moderation");
    await page.getByRole("button", { name: "コンテンツを直接削除" }).click();
    const dialog = page.getByRole("dialog");

    // Both the reference and the reason for the owner are required.
    await dialog.getByRole("button", { name: "削除する" }).click();
    await expect(dialog.getByText("理由を入力してね")).toBeVisible();
    await expect(dialog.getByText("対象の種類とIDを入力してね")).toBeVisible();

    // Something that is not an ID of anything is refused by the server.
    await dialog.getByLabel("対象の種類").selectOption({ label: "ノート" });
    await dialog.getByLabel("対象のID").fill("not-an-id");
    await dialog.getByLabel("ユーザーに伝える理由").fill(reason);
    await dialog.getByRole("button", { name: "削除する" }).click();
    await expect(page.getByText("対象が見つかりません。非公開のコンテンツはUUIDで指定してね")).toBeVisible();

    await dialog.getByLabel("対象のID").fill(subjectId);
    await dialog.getByLabel("内部メモ").fill(note);
    await dialog.getByRole("button", { name: "削除する" }).click();
    await expect(page.getByText("コンテンツを削除しました")).toBeVisible();
    await expect(dialog).toHaveCount(0);

    await page.getByRole("link", { name: "対応ログ" }).click();
    await expect(page).toHaveURL(/\/admin\/moderation\/log$/);
    await page.getByLabel("対応", { exact: true }).selectOption({ label: "コンテンツを削除" });
    await expect(page.getByText(reason)).toBeVisible();
    await expect(page.getByText(note)).toBeVisible();

    const log = await rpc<{ actions?: { subject?: { id: string }; reason: string }[] }>(request, `${MODERATION}/ListModerationActions`, { moderatorId: admin.userId, kind: "MODERATION_ACTION_KIND_REMOVE_CONTENT" }, admin);
    expect((log.actions ?? []).find((action) => action.reason === reason)?.subject?.id).toBe(subjectId);
  });

  test("a reported account becomes a case that is taken, annotated, resolved and reopened", async ({ page, request }) => {
    const admin = await asAdmin(page, request);
    const owner = await createAccount(request, "reported");
    const reporter = await createAccount(request, "reporter");
    const text = `e2e: なりすましだと思います ${tag()}`;

    // Cases come from reports filed through the engagement domain.
    try {
      await rpc(request, "tokuly.engagement.v1.ReportService/CreateReport", { subject: { type: "SUBJECT_TYPE_USER", id: owner.userId }, category: "REPORT_CATEGORY_IMPERSONATION", text }, reporter);
    } catch (error) {
      test.skip(error instanceof RpcError && error.status === 404, "the engagement domain (reports) is not served by this stack yet");
      throw error;
    }
    let caseId = "";
    await expect(async () => {
      const queue = await rpc<{ cases?: { id: string }[] }>(request, `${MODERATION}/ListCases`, { ownerId: owner.userId }, admin);
      caseId = queue.cases?.[0]?.id ?? "";
      expect(caseId).not.toBe("");
    }).toPass({ timeout: 20_000 });

    await page.goto(`/admin/moderation?owner=${owner.userId}`);
    // The row also links to the reported account; the case is the link that leads to it.
    await page.locator(`a[href*="/admin/moderation/${caseId}"]`).first().click();
    await expect(page).toHaveURL(new RegExp(`/admin/moderation/${caseId}`));
    await expect(page.getByRole("region", { name: /^通報/ }).getByText(text)).toBeVisible();

    const assignment = page.getByRole("region", { name: "担当", exact: true });
    await assignment.getByRole("button", { name: "自分が担当する" }).click();
    await expect(assignment.getByText(`@${admin.handle}`)).toBeVisible();

    const notes = page.getByRole("region", { name: "メモ" });
    await notes.getByLabel("メモを追加").fill("e2e: プロフィールを確認した");
    await notes.getByRole("button", { name: "メモを追加" }).click();
    await expect(notes.getByText("e2e: プロフィールを確認した")).toBeVisible();

    const resolve = page.getByRole("region", { name: "このケースを解決する" });
    await resolve.getByRole("radio", { name: /^警告/ }).click();
    await resolve.getByLabel("ユーザーに伝える理由").fill("e2e: 他の人になりすまさないでね");
    await resolve.getByRole("button", { name: "この内容で解決する" }).click();
    await page.getByRole("dialog").getByRole("button", { name: "解決する" }).click();
    await expect(page.getByText("ケースを解決しました！")).toBeVisible();
    await expect(page.getByRole("region", { name: "対応結果" }).getByText("警告", { exact: true })).toBeVisible();
    await expect(page.getByRole("region", { name: "対応履歴" }).getByText("e2e: 他の人になりすまさないでね")).toBeVisible();

    await page.getByRole("button", { name: "ケースを再オープン" }).click();
    await page.getByRole("dialog").getByRole("button", { name: "再オープンする" }).click();
    await expect(page.getByText("ケースをキューにもどしました")).toBeVisible();
    await expect(page.getByRole("region", { name: "このケースを解決する" })).toBeVisible();
  });
});

test.describe("audit log", () => {
  test("lists what staff did, filtered by action and account, with the change spelled out", async ({ page, request }) => {
    const admin = await asAdmin(page, request);
    const target = await createAccount(request, "audit");
    const note = `e2e: 本人確認ずみ ${tag()}`;
    await rpc(request, `${USERS}/SetUserVerified`, { userId: target.userId, verified: true, note }, admin);
    await rpc(request, `${USERS}/WarnUser`, { userId: target.userId, reason: "e2e: 監査ログ用の警告" }, admin);

    // From the account's page: everything that affected this account.
    await page.goto(`/admin/users/${target.userId}`);
    await page.getByRole("link", { name: "監査ログを見る" }).click();
    await expect(page).toHaveURL(new RegExp(`/admin/audit\\?targetUser=${target.userId}`));
    await expect(page.getByText("user.verified_changed")).toBeVisible();
    await expect(page.getByText("user.warned")).toBeVisible();

    // Narrowed to one action, the other entry leaves.
    await page.getByLabel("操作", { exact: true }).selectOption("user.verified_changed");
    await expect(page).toHaveURL(/action=user\.verified_changed/);
    await expect(page.getByText("user.warned")).toHaveCount(0);
    await expect(page.getByText(note).first()).toBeVisible();

    // Expanded, the entry shows what changed.
    await page.getByRole("button", { name: "user.verified_changed の詳細を開く" }).click();
    const change = page.getByRole("table", { name: "詳細" }).getByRole("row").filter({ hasText: "verified" });
    await expect(change.getByRole("cell").nth(0)).toHaveText("false");
    await expect(change.getByRole("cell").nth(1)).toHaveText("true");

    // A window that ends in the past holds nothing.
    await page.getByLabel("この日時まで").fill("2001-01-01T00:00");
    await expect(page.getByText("この条件の記録はありません")).toBeVisible();
    await page.getByRole("button", { name: "絞り込みをリセット" }).click();
    await expect(page).toHaveURL(/\/admin\/audit$/);
    await expect(page.getByText("この条件の記録はありません")).toHaveCount(0);
  });
});
