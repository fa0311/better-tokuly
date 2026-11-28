import { createHmac } from "node:crypto";
import { crc32, deflateSync } from "node:zlib";
import { toBinary } from "@bufbuild/protobuf";
import type { APIRequestContext, Page } from "@playwright/test";
import { create, identity } from "@tokuly/api";
import {
  type Account,
  backOnline,
  expect as baseExpect,
  createAccount,
  failToLoadStrings,
  mailedVerificationToken,
  mailpitIsUp,
  PASSWORD,
  rpc,
  signInAs,
  test,
} from "./support";

// These flows hash passwords, process images and reload pages; on a busy
// machine that takes a while, so they get more room than the defaults.
test.describe.configure({ timeout: 120_000 });
const expect = baseExpect.configure({ timeout: 25_000 });

const AUTH = "tokuly.identity.v1.AuthService";
const ACCOUNT = "tokuly.identity.v1.AccountService";
const FRIENDS = "tokuly.social.v1.FriendService";

/** From this width the section list sits beside the open section. */
const isWide = (page: Page) => (page.viewportSize()?.width ?? 0) >= 1024;

const heading = (page: Page, name: string) => page.getByRole("heading", { level: 1, name });

/** A toast with this message (the same words can also be on the page, in the security log for one). */
const toast = (page: Page, message: string) =>
  page.locator("[data-sonner-toast]").filter({ hasText: message });

/** The card with the signed-in person: in the side pane on wide screens, on the settings home on phones. */
async function identityCard(page: Page) {
  if (!isWide(page)) await page.getByRole("link", { name: "back" }).click();
  return page.getByRole("link", { name: /プロフィールを見る/ });
}

/** The current TOTP code for a base32 secret (RFC 6238: HMAC-SHA1, 30 s steps, 6 digits). */
function totpCode(secret: string, at = Date.now()): string {
  const alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
  const bits = [...secret.replace(/=+$/, "").toUpperCase()]
    .map((char) => alphabet.indexOf(char).toString(2).padStart(5, "0"))
    .join("");
  const key = Buffer.from((bits.match(/.{8}/g) ?? []).map((byte) => Number.parseInt(byte, 2)));
  const counter = Buffer.alloc(8);
  counter.writeBigUInt64BE(BigInt(Math.floor(at / 30_000)));
  const mac = createHmac("sha1", key).update(counter).digest();
  const offset = (mac.at(-1) ?? 0) & 0xf;
  return String((mac.readUInt32BE(offset) & 0x7fffffff) % 1_000_000).padStart(6, "0");
}

/** A PNG with a colour gradient, so a crop of it is still a real picture. */
function png(width: number, height: number): Buffer {
  const row = width * 3 + 1;
  const pixels = Buffer.alloc(row * height);
  for (let y = 0; y < height; y++) {
    for (let x = 0; x < width; x++)
      pixels.set(
        [255 - Math.round((x / width) * 200), 90 + Math.round((y / height) * 120), 200],
        y * row + 1 + x * 3,
      );
  }
  const chunk = (type: string, data: Buffer) => {
    const body = Buffer.concat([Buffer.from(type), data]);
    const framed = Buffer.alloc(body.length + 8);
    framed.writeUInt32BE(data.length, 0);
    body.copy(framed, 4);
    framed.writeUInt32BE(crc32(body), body.length + 4);
    return framed;
  };
  const header = Buffer.alloc(13);
  header.writeUInt32BE(width, 0);
  header.writeUInt32BE(height, 4);
  header.set([8, 2], 8); // 8 bits per channel, RGB
  return Buffer.concat([
    Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]),
    chunk("IHDR", header),
    chunk("IDAT", deflateSync(pixels)),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}

/**
 * Signs in through the login form. The page is server-rendered, so typing has
 * to wait until the app runs in the browser: that is the moment it has found
 * out that nobody is signed in and dropped its "was signed in" note.
 */
async function submitLoginForm(page: Page, identifier: string, password: string): Promise<void> {
  await page.goto("/login");
  await page.waitForFunction(() => window.localStorage.getItem("tokuly.signedIn") === null);
  await expect(page.getByLabel("メールアドレス または ハンドル")).toHaveCount(1);
  await page.getByLabel("メールアドレス または ハンドル").fill(identifier);
  await page.getByLabel("パスワード").fill(password);
  await page.getByRole("button", { name: "ログイン", exact: true }).click();
}

const getMe = async (request: APIRequestContext, as: Account) =>
  (await rpc<{ me: Record<string, unknown> }>(request, `${ACCOUNT}/GetMe`, {}, as)).me;
const getSettings = async (request: APIRequestContext, as: Account) =>
  (
    await rpc<{
      settings: { privacy: Record<string, unknown>; appearance: Record<string, unknown> };
    }>(request, `${ACCOUNT}/GetSettings`, {}, as)
  ).settings;

// ── Home ────────────────────────────────────────────────────────────

test("the settings home lists every section and opens each of them", async ({ page, me: _me }) => {
  await page.goto("/settings");
  const sections = page.getByRole("navigation", { name: "設定" });
  for (const [title, path] of [
    ["プロフィール", "/settings/profile"],
    ["プロフ帳", "/settings/profile-book"],
    ["アカウント", "/settings/account"],
    ["アカウント切り替え", "/settings/accounts"],
    ["セキュリティ", "/settings/security"],
    ["プライバシー", "/settings/privacy"],
    ["見た目", "/settings/appearance"],
    ["連携サービス", "/settings/connections"],
    ["あなたのデータ", "/settings/data"],
  ] as const) {
    // The row's name is its title and then its hint ("アカウント" is not "アカウント切り替え").
    const name = new RegExp(`^${title}(?!切り替え)`);
    await sections.getByRole("link", { name }).click();
    await expect(page).toHaveURL(new RegExp(`${path}$`));
    await expect(heading(page, title)).toBeVisible();
    // Phones show one thing at a time; wide screens keep the list next to the section.
    if (!isWide(page)) await page.getByRole("link", { name: "back" }).click();
  }
});

test("a row whose strings cannot load shows no raw keys, and its words come once they arrive", async ({
  page,
  me,
}) => {
  // The licenses' strings fail to arrive, as when the connection drops right as settings open.
  const letThrough = await failToLoadStrings(page, "licenses");
  await page.goto(`/u/${me.handle}`);
  await page.getByRole("link", { name: "設定" }).click();
  const sections = page.getByRole("navigation", { name: "設定" });
  await expect(sections.getByRole("link", { name: /^プロフィール/ })).toBeVisible();

  await letThrough();
  await expect(sections).not.toContainText("settingsRow");
  await backOnline(page);
  await expect(sections.getByRole("link", { name: /^ライセンス表記/ })).toBeVisible();
  await expect(sections).not.toContainText("settingsRow");
});

test("the licenses credit the map and place data, each with its licence", async ({
  page,
  me: _me,
}) => {
  await page.goto("/settings");
  await page
    .getByRole("navigation", { name: "設定" })
    .getByRole("link", { name: /^ライセンス表記/ })
    .click();
  await expect(heading(page, "ライセンス表記")).toBeVisible();
  for (const [name, license] of [
    ["GeoNames", "CC-BY-4.0"],
    ["OpenStreetMap", "ODbL-1.0"],
    ["M PLUS Rounded 1c", "OFL-1.1"],
  ]) {
    const row = page.getByRole("listitem").filter({ has: page.getByRole("link", { name }) });
    await expect(row.getByRole("link", { name: license })).toHaveAttribute("href", /^https:/);
  }
});

// ── Profile ─────────────────────────────────────────────────────────

test("an edited profile shows at once in the app and on the profile page", async ({ page, me }) => {
  await page.goto("/settings/profile");
  await page.getByLabel("なまえ").fill("あたらしい なまえ");
  await page.getByLabel("ひとこと").fill("ラーメンと散歩がすき！");
  await page.getByLabel("学校").fill("とくり高校");
  await page.getByRole("button", { name: "保存する" }).click();
  await expect(page.getByText("プロフィールを保存しました！")).toBeVisible();

  // No reload: the rest of the app reads the same signed-in account.
  const card = await identityCard(page);
  await expect(card).toContainText("あたらしい なまえ");
  await card.click();
  await expect(page).toHaveURL(new RegExp(`/u/${me.handle}$`));
  await expect(page.getByRole("heading", { name: /あたらしい なまえ/ })).toBeVisible();
  await expect(page.getByText("ラーメンと散歩がすき！")).toBeVisible();

  // The school is stored too, and the form shows it when it is opened again.
  await page.goto("/settings/profile");
  await expect(page.getByLabel("学校")).toHaveValue("とくり高校");
  await expect(page.getByLabel("ひとこと")).toHaveValue("ラーメンと散歩がすき！");
});

test("a handle that is taken, reserved or malformed is refused with the reason; a free one is saved", async ({
  page,
  me,
  request,
}) => {
  const other = await createAccount(request, "taken");
  await page.goto("/settings/profile");
  const handle = page.getByLabel("ハンドル");
  const save = page.getByRole("button", { name: "保存する" });

  await handle.fill(other.handle);
  await expect(page.getByText("このハンドルはもう使われています")).toBeVisible();
  await expect(save).toBeDisabled();

  await handle.fill("admin");
  await expect(page.getByText("このハンドルは使えません")).toBeVisible();

  await handle.fill("ab");
  await expect(page.getByText("3〜24文字の a-z、0-9、_、- で、最初は英数字にしてね")).toBeVisible();
  await expect(save).toBeDisabled();

  const fresh = `${me.handle.slice(0, 14)}_new`;
  await handle.fill(fresh);
  await expect(page.getByText("このハンドルは使えます！")).toBeVisible();
  await save.click();
  await expect(page.getByText("プロフィールを保存しました！")).toBeVisible();

  await page.goto(`/u/${fresh}`);
  await expect(page.getByText(`@${fresh}`)).toBeVisible();
});

test("a status is shared with a tap, shows on the profile, and can be cleared", async ({
  page,
  me,
}) => {
  await page.goto("/settings/profile");
  await page.getByRole("button", { name: "設定する" }).click();
  const dialog = page.getByRole("dialog", { name: "いま、なにしてる？" });
  await dialog.getByRole("button", { name: /ごはん中/ }).click();
  await dialog.getByText("1時間").click();
  await dialog.getByRole("button", { name: "シェアする" }).click();
  await expect(page.getByTestId("status-pill")).toContainText("ごはん中");
  await expect(page.getByText(/あと.+で消えます/)).toBeVisible();

  await page.goto(`/u/${me.handle}`);
  await expect(page.getByText("ごはん中")).toBeVisible();

  await page.goto("/settings/profile");
  await page.getByRole("button", { name: "変える", exact: true }).click();
  await page.getByRole("dialog").getByRole("button", { name: "クリア" }).click();
  await expect(page.getByText("いまは設定されていません")).toBeVisible();
});

test("a picked picture is cropped, uploaded and becomes the avatar; it can be removed again", async ({
  page,
  me: _me,
}) => {
  await page.goto("/settings/profile");
  const chooser = page.waitForEvent("filechooser");
  await page.getByRole("button", { name: "アイコンを変える" }).click();
  await (await chooser).setFiles({ name: "me.png", mimeType: "image/png", buffer: png(600, 400) });

  const crop = page.getByRole("dialog", { name: "アイコンを切り抜く" });
  await expect(
    crop.getByRole("application", { name: "切り抜く範囲" }).locator("img"),
  ).toBeVisible();
  // Zoom in a little with the keyboard, as someone adjusting the frame would.
  await crop.getByRole("slider", { name: "拡大" }).press("ArrowRight");
  await crop.getByRole("button", { name: "これにする！" }).click();
  await expect(page.getByText("アイコンを変えました！")).toBeVisible({ timeout: 30_000 });
  const pictures = page.getByTestId("profile-images");
  await expect(pictures.locator('img[src*="/media/"]')).toBeVisible();

  await page.getByRole("button", { name: "アイコンを削除" }).click();
  await page
    .getByRole("dialog", { name: "アイコンを削除する？" })
    .getByRole("button", { name: "削除する" })
    .click();
  await expect(page.getByText("アイコンを削除しました")).toBeVisible();
  await expect(pictures.locator("img")).toHaveCount(0);
});

test("a banner is cropped to a wide strip and shown on the profile", async ({ page, me }) => {
  await page.goto("/settings/profile");
  const chooser = page.waitForEvent("filechooser");
  await page.getByRole("button", { name: "バナーを変える" }).click();
  await (await chooser).setFiles({
    name: "banner.png",
    mimeType: "image/png",
    buffer: png(900, 600),
  });
  await page
    .getByRole("dialog", { name: "バナーを切り抜く" })
    .getByRole("button", { name: "これにする！" })
    .click();
  await expect(page.getByText("バナーを変えました！")).toBeVisible({ timeout: 30_000 });
  await expect(page.getByTestId("banner-preview")).toHaveCSS("background-image", /\/media\//);

  // The stored image has the banner's 3:1 shape.
  const banner = (await getMe(page.request, me)).banner as { width: number; height: number };
  expect(banner.width / banner.height).toBeCloseTo(3, 1);
});

// ── Account ─────────────────────────────────────────────────────────

test("after changing the password, the new one signs in and the old one does not", async ({
  page,
  me,
  request,
}) => {
  const newPassword = "even better horse 42";
  await page.goto("/settings/account");

  // A wrong current password is pointed out at its field.
  await page.getByLabel("いまのパスワード").fill("not my password");
  await page.getByLabel("新しいパスワード").fill(newPassword);
  await page.getByRole("button", { name: "パスワードを変える" }).click();
  await expect(page.getByText("いまのパスワードがちがいます")).toBeVisible();

  await page.getByLabel("いまのパスワード").fill(PASSWORD);
  await page.getByRole("button", { name: "パスワードを変える" }).click();
  await expect(page.getByText(/パスワードを変えました！/)).toBeVisible();

  await page.goto("/settings/data");
  await page.getByRole("button", { name: "このデバイスからログアウト" }).click();
  await expect(page).toHaveURL(/\/$/);

  await submitLoginForm(page, me.handle, newPassword);
  await expect(page).toHaveURL(/\/map$/);

  await expect(
    rpc(request, `${AUTH}/SignIn`, { identifier: me.handle, password: PASSWORD }),
  ).rejects.toThrow(/INVALID_CREDENTIALS/);
});

test("the verification mail can be sent again, and its link marks the email as verified", async ({
  page,
  me,
  request,
}) => {
  test.skip(!(await mailpitIsUp(request)), "needs the development mail catcher (Mailpit)");
  await page.goto("/settings/account");
  await expect(page.getByText("未確認", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "確認メールをもう一度送る" }).click();
  await expect(page.getByText(`${me.email} に確認メールを送りました！`)).toBeVisible();

  // Opening the mailed link is the sign-in pages' part; here the link's token is redeemed directly.
  await rpc(request, `${AUTH}/VerifyEmail`, {
    token: await mailedVerificationToken(request, me.email),
  });

  await page.reload();
  await expect(page.getByText("確認済み", { exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "確認メールをもう一度送る" })).toHaveCount(0);
});

test("a new email address takes over once its verification link is opened", async ({
  page,
  me,
  request,
}) => {
  test.skip(!(await mailpitIsUp(request)), "needs the development mail catcher (Mailpit)");
  const newEmail = `new-${me.handle}@e2e.tokuly.test`;
  // Only a verified address is offered an undo when the account moves away.
  await rpc(request, `${AUTH}/VerifyEmail`, {
    token: await mailedVerificationToken(request, me.email),
  });
  await page.goto("/settings/account");
  await page.getByRole("button", { name: "メールアドレスを変える" }).click();
  const dialog = page.getByRole("dialog", { name: "メールアドレスを変える" });

  // The same address again is refused at the field.
  await dialog.getByLabel("新しいメールアドレス").fill(me.email);
  await dialog.getByLabel("いまのパスワード").fill(PASSWORD);
  await dialog.getByRole("button", { name: "確認メールを送る" }).click();
  await expect(dialog.getByText("いまと同じメールアドレスです")).toBeVisible();

  await dialog.getByLabel("新しいメールアドレス").fill(newEmail);
  await dialog.getByRole("button", { name: "確認メールを送る" }).click();
  await expect(page.getByRole("status").filter({ hasText: newEmail })).toBeVisible();
  // Until the link is opened the old address stays.
  await expect(page.getByTestId("account-email")).toHaveText(me.email);

  // Opened in this browser, where the account is signed in: this session
  // stays (every other device is signed out by the change).
  // The link waits for a press (mail scanners open links by themselves).
  const change = await mailedVerificationToken(request, newEmail);
  await page.goto(`/verify-email?token=${encodeURIComponent(change)}`);
  await page.getByRole("button", { name: "メールアドレスを確認する" }).click();
  await expect(page.getByText("メールアドレスを確認しました！")).toBeVisible();
  await page.goto("/settings/account");
  await expect(page.getByTestId("account-email")).toHaveText(newEmail);
  await expect(page.getByText("確認済み", { exact: true })).toBeVisible();

  // The old address is told, and its link moves the account back, signing out everywhere.
  const undo = await mailedVerificationToken(
    request,
    me.email,
    "メールアドレスが変更されました",
    "undo-email-change",
  );
  await page.goto(`/undo-email-change?token=${encodeURIComponent(undo)}`);
  await expect(page.getByText("メールアドレスを元に戻す？")).toBeVisible();
  // Opening the page did nothing yet.
  const unchanged = await rpc<{ me: { email: string } }>(request, `${AUTH}/SignIn`, {
    identifier: me.handle,
    password: PASSWORD,
  });
  expect(unchanged.me.email).toBe(newEmail);
  await page.getByRole("button", { name: "元のアドレスに戻す" }).click();
  await expect(page.getByText("メールアドレスを元に戻したよ！")).toBeVisible();
  await expect(page.getByRole("link", { name: "パスワードを再設定する" })).toBeVisible();
  const account = await rpc<{ me: { email: string } }>(request, `${AUTH}/SignIn`, {
    identifier: me.handle,
    password: PASSWORD,
  });
  expect(account.me.email).toBe(me.email);
});

test("the language switches at once, survives a reload and is stored on the account", async ({
  page,
  me,
}) => {
  await page.goto("/settings/account");
  await page.getByRole("combobox", { name: "ことば" }).click();
  await page.getByRole("option", { name: "English" }).click();
  await expect(heading(page, "Account")).toBeVisible();
  await expect.poll(async () => (await getMe(page.request, me)).locale).toBe("en");

  await page.reload();
  await expect(heading(page, "Account")).toBeVisible();
  await expect(page.getByRole("combobox", { name: "Language" })).toContainText("English");
});

test("the time zone is picked from a searchable list, or set back to the device's", async ({
  page,
  me,
}) => {
  await page.goto("/settings/account");
  await expect(page.getByTestId("account-time-zone")).toHaveText("Asia/Tokyo");
  await page.getByRole("button", { name: "変える", exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "タイムゾーンをえらぶ" });
  await dialog.getByPlaceholder(/都市名でさがす/).fill("london");
  await dialog.getByRole("button", { name: /Europe\/London/ }).click();
  await expect(page.getByText("タイムゾーンを変えました！")).toBeVisible();
  await expect(page.getByTestId("account-time-zone")).toHaveText("Europe/London");
  expect((await getMe(page.request, me)).timeZone).toBe("Europe/London");

  // The browser of the test runs in Tokyo.
  await page.getByRole("button", { name: /このデバイスに合わせる/ }).click();
  await expect(page.getByTestId("account-time-zone")).toHaveText("Asia/Tokyo");
});

// ── Security ────────────────────────────────────────────────────────

test("two-step verification is set up with a code from the secret, and signing in then asks for one", async ({
  page,
  me,
}) => {
  await page.goto("/settings/security");
  await page.getByRole("button", { name: "オンにする" }).click();
  const setup = page.getByRole("dialog", { name: "2段階認証を設定" });
  await expect(setup.getByRole("img", { name: "認証アプリ用のQRコード" })).toBeVisible();
  const secret = ((await setup.getByTestId("totp-secret").textContent()) ?? "").trim();

  // It takes the password, so a session alone cannot turn it on.
  const code = setup.getByLabel(/6桁のコードを入力/);
  const password = setup.getByLabel("いまのパスワード");
  await code.fill(totpCode(secret));
  await password.fill("not the password");
  await setup.getByRole("button", { name: "オンにする" }).click();
  await expect(setup.getByText("いまのパスワードがちがいます")).toBeVisible();

  // A wrong code is refused and the setup stays open.
  await password.fill(PASSWORD);
  await code.fill(totpCode(secret) === "000000" ? "111111" : "000000");
  await setup.getByRole("button", { name: "オンにする" }).click();
  await expect(setup.getByText("認証コードがちがいます")).toBeVisible();

  await code.fill(totpCode(secret));
  await setup.getByRole("button", { name: "オンにする" }).click();
  const recovery = page.getByRole("dialog", { name: "リカバリーコードを保存してね" });
  await expect(recovery.getByTestId("recovery-codes").getByRole("listitem")).toHaveCount(10);
  await recovery.getByRole("button", { name: "保存しました" }).click();
  await expect(page.getByText("オン", { exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "オフにする" })).toBeVisible();

  // Sign out, then back in: the password alone is no longer enough. Every
  // code works once, so this takes the next one (the server allows a step of skew).
  await page.goto("/settings/data");
  await page.getByRole("button", { name: "このデバイスからログアウト" }).click();
  await expect(page).toHaveURL(/\/$/);
  await submitLoginForm(page, me.handle, PASSWORD);
  await expect(page.getByRole("heading", { name: "認証コードを入力" })).toBeVisible();
  await page.getByLabel("認証コード").fill(totpCode(secret, Date.now() + 30_000));
  await page.getByRole("button", { name: "ログイン", exact: true }).click();
  await expect(page).toHaveURL(/\/map$/);
});

test("two-step verification is turned off with a current code", async ({ page, me, request }) => {
  const { secret } = await rpc<{ secret: string }>(request, `${ACCOUNT}/BeginTotpSetup`, {}, me);
  await rpc(
    request,
    `${ACCOUNT}/EnableTotp`,
    { code: totpCode(secret), currentPassword: PASSWORD },
    me,
  );

  await page.goto("/settings/security");
  await page.getByRole("button", { name: "オフにする" }).click();
  const dialog = page.getByRole("dialog", { name: "2段階認証をオフにする" });
  await dialog.getByLabel("認証コード").fill("000000");
  await dialog.getByRole("button", { name: "オフにする" }).click();
  await expect(dialog.getByText("認証コードがちがいます")).toBeVisible();

  // The code that turned it on is spent; the next one is good.
  await dialog.getByLabel("認証コード").fill(totpCode(secret, Date.now() + 30_000));
  await dialog.getByRole("button", { name: "オフにする" }).click();
  await expect(toast(page, "2段階認証をオフにしました")).toBeVisible();
  await expect(page.getByRole("button", { name: "オンにする" })).toBeVisible();
  expect((await getMe(request, me)).totpEnabled ?? false).toBe(false);
});

test("a passkey is added with a name, listed, and removed", async ({ page, me }) => {
  // A software authenticator that confirms every request, standing in for Touch ID and friends.
  const devtools = await page.context().newCDPSession(page);
  await devtools.send("WebAuthn.enable");
  await devtools.send("WebAuthn.addVirtualAuthenticator", {
    options: {
      protocol: "ctap2",
      transport: "internal",
      hasResidentKey: true,
      hasUserVerification: true,
      isUserVerified: true,
      automaticPresenceSimulation: true,
    },
  });

  await page.goto("/settings/security");
  await expect(page.getByText("まだパスキーがありません")).toBeVisible();
  await page.getByRole("button", { name: "パスキーを追加" }).click();
  const dialog = page.getByRole("dialog", { name: "パスキーを追加" });
  await dialog.getByLabel("パスキーのなまえ").fill("テスト用のキー");
  // A new way to sign in takes the password, not just this session.
  await dialog.getByLabel("いまのパスワード").fill("not the password");
  await dialog.getByRole("button", { name: "登録する" }).click();
  await expect(dialog.getByText("いまのパスワードがちがいます")).toBeVisible();
  await dialog.getByLabel("いまのパスワード").fill(PASSWORD);
  await dialog.getByRole("button", { name: "登録する" }).click();
  await expect(toast(page, "パスキーを追加しました！")).toBeVisible();
  await expect(page.getByRole("main").getByText("テスト用のキー")).toBeVisible();
  expect((await getMe(page.request, me)).passkeyCount).toBe(1);

  await page.getByRole("button", { name: "「テスト用のキー」を削除" }).click();
  await page
    .getByRole("dialog", { name: "「テスト用のキー」を削除する？" })
    .getByRole("button", { name: "削除する" })
    .click();
  await expect(toast(page, "パスキーを削除しました")).toBeVisible();
  await expect(page.getByText("まだパスキーがありません")).toBeVisible();
});

/**
 * Answers GetAuthCapabilities as a server reached by an IP address does
 * (`TOKULY_PUBLIC_URL=http://192.168.1.10`): WebAuthn needs a domain name,
 * and this test server runs on localhost, which has one.
 */
async function serveWithoutPasskeys(page: Page): Promise<void> {
  await page.route(`**/${AUTH}/GetAuthCapabilities`, (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/proto",
      body: Buffer.from(
        toBinary(
          identity.GetAuthCapabilitiesResponseSchema,
          create(identity.GetAuthCapabilitiesResponseSchema, {
            capabilities: { passkeysAvailable: false },
          }),
        ),
      ),
    }),
  );
}

test("the sign-in screen offers a passkey only when the server has passkeys", async ({ page }) => {
  await page.goto("/login");
  await expect(page.getByRole("button", { name: "パスキーでログイン" })).toBeVisible();

  await serveWithoutPasskeys(page);
  await page.goto("/login");
  await page.waitForFunction(() => window.localStorage.getItem("tokuly.signedIn") === null);
  await expect(page.getByRole("button", { name: "ログイン", exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "パスキーでログイン" })).toHaveCount(0);
});

test("on a server without passkeys, settings say so instead of offering to add one", async ({
  page,
  me: _me,
}) => {
  await serveWithoutPasskeys(page);
  await page.goto("/settings/security");
  await expect(
    page.getByText("このサーバーではパスキーが使えません。", { exact: false }),
  ).toBeVisible();
  await expect(page.getByRole("button", { name: "パスキーを追加" })).toHaveCount(0);
});

test("signed-in devices are listed with this one marked, and another one can be signed out", async ({
  page,
  me,
  request,
}) => {
  // A second sign-in from a phone.
  await rpc(request, `${AUTH}/SignIn`, {
    identifier: me.handle,
    password: PASSWORD,
    client: { deviceName: "Pixel 10", platform: "PLATFORM_ANDROID" },
  });

  await page.goto("/settings/security");
  const rows = page.getByTestId("session-row");
  await expect(rows.filter({ hasText: "このデバイス" })).toHaveCount(1);
  const phone = rows.filter({ hasText: "Pixel 10" });
  await expect(phone).toBeVisible();

  await phone.getByRole("button", { name: "ログアウト" }).click();
  await page
    .getByRole("dialog", { name: "「Pixel 10」をログアウトさせる？" })
    .getByRole("button", { name: "ログアウトさせる" })
    .click();
  await expect(toast(page, "ログアウトさせました")).toBeVisible();
  await expect(phone).toHaveCount(0);

  const { sessions } = await rpc<{ sessions: { client?: { deviceName?: string } }[] }>(
    request,
    `${ACCOUNT}/ListSessions`,
    {},
    me,
  );
  expect(sessions.map((session) => session.client?.deviceName)).not.toContain("Pixel 10");
});

test("the security log tells what happened to the account", async ({ page, me, request }) => {
  await rpc(request, `${AUTH}/SignIn`, {
    identifier: me.handle,
    password: "definitely wrong",
  }).catch(() => {});

  await page.goto("/settings/security");
  await page.getByRole("link", { name: "すべて見る" }).click();
  await expect(page).toHaveURL(/\/settings\/security\/log$/);
  const events = page.getByTestId("security-event");
  await expect(events.filter({ hasText: "アカウントをつくりました" })).toHaveCount(1);
  await expect(events.filter({ hasText: "ログインに失敗しました" })).toHaveCount(1);
  await expect(events.filter({ hasText: "ログインしました" }).first()).toBeVisible();
});

test("a sign-in on another device is approved by its code", async ({ page, me, request }) => {
  const started = await rpc<{ deviceCode: string; userCode: string }>(
    request,
    `${AUTH}/StartDeviceLogin`,
    {
      client: { deviceName: "Tokuly Studio", platform: "PLATFORM_DESKTOP", appVersion: "1.2.3" },
    },
  );

  // The code is typed in, as from a TV screen.
  await page.goto("/settings/security");
  await page.getByRole("link", { name: /別のデバイスのログインを承認/ }).click();
  await page.getByLabel("コード").fill(started.userCode.toLowerCase());
  await page.getByRole("button", { name: "確認する" }).click();

  await expect(page.getByTestId("device-name")).toHaveText("Tokuly Studio");
  await expect(page.getByTestId("device-code")).toHaveText(started.userCode);
  await page.getByRole("button", { name: "承認する" }).click();
  await expect(page.getByText("承認しました！")).toBeVisible();

  const poll = await rpc<{ status: string; auth?: { me: { user: { handle: string } } } }>(
    request,
    `${AUTH}/PollDeviceLogin`,
    { deviceCode: started.deviceCode },
  );
  expect(poll.status).toBe("DEVICE_LOGIN_STATUS_APPROVED");
  expect(poll.auth?.me.user.handle).toBe(me.handle);
});

test("a device sign-in from a link can be refused, and a wrong code says so", async ({
  page,
  me: _me,
  request,
}) => {
  const started = await rpc<{ deviceCode: string; userCode: string }>(
    request,
    `${AUTH}/StartDeviceLogin`,
    { client: { deviceName: "Living room TV" } },
  );

  // The address a device shows (or encodes as a QR code) carries the code.
  await page.goto(`/settings/security/device?code=${started.userCode}`);
  await expect(page.getByTestId("device-name")).toHaveText("Living room TV");
  await page.getByRole("button", { name: "拒否する" }).click();
  await expect(page.getByText("拒否しました")).toBeVisible();
  const poll = await rpc<{ status: string }>(request, `${AUTH}/PollDeviceLogin`, {
    deviceCode: started.deviceCode,
  });
  expect(poll.status).toBe("DEVICE_LOGIN_STATUS_DENIED");

  await page.getByRole("button", { name: "別のコードを入力" }).click();
  await page.getByLabel("コード").fill("ZZZZ-9999");
  await page.getByRole("button", { name: "確認する" }).click();
  await expect(page.getByText("このコードは無効か、期限が切れています")).toBeVisible();
});

// ── Privacy ─────────────────────────────────────────────────────────

test("a privacy switch saves the moment it is flipped and stays that way", async ({ page, me }) => {
  await page.goto("/settings/privacy");
  const battery = page.getByRole("switch", { name: /バッテリー残量/ });
  await expect(battery).toBeChecked();
  await battery.click();
  await expect(battery).not.toBeChecked();
  await expect
    .poll(async () => (await getSettings(page.request, me)).privacy.shareBattery ?? false)
    .toBe(false);
  // Only that one setting changed.
  expect((await getSettings(page.request, me)).privacy.shareActivity).toBe(true);

  await page.reload();
  await expect(page.getByRole("switch", { name: /バッテリー残量/ })).not.toBeChecked();
  await expect(page.getByRole("switch", { name: /スピードと移動手段/ })).toBeChecked();
});

test("the next person to sign in on the same browser gets their own settings, not the last person's", async ({
  page,
  me,
  request,
}) => {
  const next = await createAccount(request, "next");
  await page.goto("/settings/privacy");
  await page.getByRole("switch", { name: /バッテリー残量/ }).click();
  await expect
    .poll(async () => (await getSettings(page.request, me)).privacy.shareBattery ?? false)
    .toBe(false);

  // Sign out and in again without ever reloading the page, as someone handing over a shared computer would.
  await (isWide(page)
    ? page.getByRole("link", { name: /^あなたのデータ/ })
    : page.getByRole("link", { name: "back" })
  ).click();
  if (!isWide(page)) await page.getByRole("link", { name: /^あなたのデータ/ }).click();
  await page.getByRole("button", { name: "このデバイスからログアウト" }).click();
  await expect(page).toHaveURL(/\/$/);
  await page.getByRole("link", { name: "ログイン" }).first().click();
  await page.getByLabel("メールアドレス または ハンドル").fill(next.handle);
  await page.getByLabel("パスワード").fill(PASSWORD);
  await page.getByRole("button", { name: "ログイン", exact: true }).click();
  await expect(page).toHaveURL(/\/map$/);

  await page.getByRole("link", { name: "じぶん" }).click();
  await page.getByRole("main").getByRole("link", { name: "設定" }).click();
  await page
    .getByRole("navigation", { name: "設定" })
    .getByRole("link", { name: /^プライバシー/ })
    .click();
  await expect(page.getByRole("switch", { name: /バッテリー残量/ })).toBeChecked();
});

test("an audience is chosen from options that each say what they mean", async ({ page, me }) => {
  await page.goto("/settings/privacy");
  const messages = page.getByRole("combobox", { name: "メッセージを送れる人" });
  await expect(messages).toContainText("ともだちだけ");
  await messages.click();
  await expect(page.getByRole("option", { name: /だれでもメッセージを送れます/ })).toBeVisible();
  await page.getByRole("option", { name: /共通のともだちがいる人まで送れます/ }).click();
  await expect(messages).toContainText("ともだちのともだちまで");
  await expect
    .poll(async () => (await getSettings(page.request, me)).privacy.directMessagesFrom)
    .toBe("AUDIENCE_FRIENDS_OF_FRIENDS");

  await page.reload();
  await expect(page.getByRole("combobox", { name: "メッセージを送れる人" })).toContainText(
    "ともだちのともだちまで",
  );
});

test("a minor starts found by friends of friends and cannot widen it", async ({
  page,
  request,
}) => {
  const teen = await createAccount(request, "teen", {
    birthdate: `${new Date().getFullYear() - 15}-04-02`,
  });
  await signInAs(page, teen);
  await page.goto("/settings/privacy");
  await expect(
    page.getByText(
      "18歳になるまでは、見つけてもらう範囲とリクエストは「ともだちのともだちまで」だよ",
    ),
  ).toBeVisible();
  for (const name of ["あなたを見つけられる人", "ともだちリクエストを送れる人"]) {
    const select = page.getByRole("combobox", { name });
    await expect(select).toContainText("ともだちのともだちまで");
    await select.click();
    await expect(page.getByRole("option", { name: /^だれにも/ })).toBeVisible();
    await expect(page.getByRole("option", { name: /^だれでも/ })).toHaveCount(0);
    await page.keyboard.press("Escape");
  }
  const notes = page.getByRole("combobox", { name: "ノートのはじめの公開範囲" });
  await expect(notes).toContainText("フォロワーだけ");
});

test("blocked people are listed and can be unblocked", async ({ page, me, request }) => {
  const other = await createAccount(request, "blocked");
  await rpc(request, `${FRIENDS}/BlockUser`, { userId: other.userId }, me);

  await page.goto("/settings/privacy");
  await page.getByRole("link", { name: /ブロックした人/ }).click();
  const row = page.getByTestId("blocked-user").filter({ hasText: `@${other.handle}` });
  await expect(row).toBeVisible();
  await row.getByRole("button", { name: "ブロックを解除" }).click();
  await page.getByRole("dialog").getByRole("button", { name: "ブロックを解除" }).click();
  await expect(page.getByText("blockedさんのブロックを解除しました")).toBeVisible();
  await expect(page.getByText("だれもブロックしていません")).toBeVisible();

  const { users = [] } = await rpc<{ users?: unknown[] }>(
    request,
    `${FRIENDS}/ListBlockedUsers`,
    {},
    me,
  );
  expect(users).toHaveLength(0);
});

// ── Appearance ──────────────────────────────────────────────────────

test("theme and ドパガキ度 apply at once and are still there after a reload", async ({
  page,
  me,
}) => {
  await page.goto("/settings/appearance");
  const html = page.locator("html");
  await expect(html).toHaveAttribute("data-theme", "light");
  await expect(html).toHaveAttribute("data-dopa", "3");

  await page.getByRole("radio", { name: "ダーク" }).click();
  await expect(html).toHaveAttribute("data-theme", "dark");

  const level = page.getByRole("slider", { name: "ドパガキ度" });
  await level.press("ArrowRight");
  await level.press("ArrowRight");
  await expect(html).toHaveAttribute("data-dopa", "5");
  await expect(page.getByTestId("dopamine-level")).toContainText("ドパガキMAX");
  // The preview says what this level does.
  await expect(page.getByTestId("dopamine-preview")).toContainText("キラキラ：13こ");

  // Calm: no motion at all.
  for (let step = 0; step < 5; step++) await level.press("ArrowLeft");
  await expect(html).toHaveAttribute("data-dopa", "0");
  await expect(page.getByTestId("dopamine-preview")).toContainText("動き：オフ");
  await level.press("ArrowRight");
  await expect(html).toHaveAttribute("data-dopa", "1");

  // Saved to the account as well, for the next device.
  await expect
    .poll(async () => (await getSettings(page.request, me)).appearance)
    .toMatchObject({ theme: "dark", dopamineLevel: 1 });

  await page.reload();
  await expect(html).toHaveAttribute("data-theme", "dark");
  await expect(html).toHaveAttribute("data-dopa", "1");
  await expect(page.getByRole("radio", { name: "ダーク" })).toBeChecked();
  await expect(page.getByRole("slider", { name: "ドパガキ度" })).toHaveValue("1");
});

test("an account's look is taken over when it signs in on a new browser", async ({
  page,
  me,
  request,
}) => {
  const { appearance } = await getSettings(request, me);
  await rpc(
    request,
    `${ACCOUNT}/UpdateSettings`,
    { appearance: { ...appearance, theme: "dark", dopamineLevel: 5, accentColor: "#ec3f8c" } },
    me,
  );

  await page.goto("/settings/appearance");
  const html = page.locator("html");
  await expect(html).toHaveAttribute("data-theme", "dark");
  await expect(html).toHaveAttribute("data-dopa", "5");
  await expect(page.getByRole("radio", { name: "ピンク" })).toBeChecked();
});

test("the accent colour recolours the app and the other preferences are saved", async ({
  page,
  me,
}) => {
  await page.goto("/settings/appearance");
  const brand = () =>
    page.evaluate(() =>
      getComputedStyle(document.documentElement).getPropertyValue("--brand").trim(),
    );
  expect(await brand()).toBe("#7859f7");
  await page.getByRole("radio", { name: "みどり" }).click();
  await expect.poll(brand).toBe("#0f8759");

  await page.getByRole("combobox", { name: "距離の単位" }).click();
  await page.getByRole("option", { name: /マイル/ }).click();
  await page.getByRole("switch", { name: /動画の自動再生/ }).click();
  await expect
    .poll(async () => (await getSettings(page.request, me)).appearance)
    .toMatchObject({ accentColor: "#12a36b", distanceUnit: "mi", sounds: true });
  expect((await getSettings(page.request, me)).appearance.autoplay ?? false).toBe(false);

  await page.reload();
  await expect.poll(brand).toBe("#0f8759");
  await expect(page.getByRole("radio", { name: "みどり" })).toBeChecked();
  await expect(page.getByRole("switch", { name: /動画の自動再生/ })).not.toBeChecked();
});

// ── Connections ─────────────────────────────────────────────────────

test("Spotify can be connected where the server is set up for it, and says so where it is not", async ({
  page,
  me,
  request,
}) => {
  const { availableProviders = [] } = await rpc<{ availableProviders?: string[] }>(
    request,
    `${ACCOUNT}/ListConnections`,
    {},
    me,
  );
  await page.goto("/settings/connections");
  const spotify = page.getByTestId("connection-spotify");
  await expect(spotify.getByText("Spotify", { exact: true })).toBeVisible();
  if (availableProviders.includes("spotify")) {
    await expect(spotify.getByRole("button", { name: "Spotifyとつなぐ" })).toBeEnabled();
  } else {
    await expect(
      spotify.getByText("このサーバーではSpotifyとの連携が設定されていません。", { exact: false }),
    ).toBeVisible();
    await expect(spotify.getByRole("button")).toHaveCount(0);
  }
});

test("coming back from a provider without agreeing leaves things as they were", async ({
  page,
  me: _me,
}) => {
  await page.goto("/settings/connections/callback/spotify?error=access_denied&state=whatever");
  await expect(page.getByText("連携をキャンセルしました")).toBeVisible();
  await page.getByRole("link", { name: "連携サービスにもどる" }).click();
  await expect(page).toHaveURL(/\/settings\/connections$/);
});

// ── Your data ───────────────────────────────────────────────────────

test("a data export is requested and the request is still shown after a reload", async ({
  page,
  me,
}) => {
  await page.goto("/settings/data");
  await page.getByRole("button", { name: "データをリクエスト" }).click();
  await expect(page.getByText("リクエストしました！メールをまっててね")).toBeVisible();
  await expect(page.getByTestId("export-state")).toContainText(me.email);

  await page.reload();
  await expect(page.getByTestId("export-state")).toContainText("リクエストしました");
});

test("deleting the account is scheduled after confirming, and can be cancelled", async ({
  page,
  me,
  request,
}) => {
  await page.goto("/settings/data");
  await page.getByRole("button", { name: "アカウントを削除する" }).click();
  const dialog = page.getByRole("dialog", { name: "ほんとうに削除する？" });
  const confirm = dialog.getByRole("button", { name: "削除を予約する" });
  await expect(confirm).toBeDisabled();

  // Both confirmations are needed: the password and the handle.
  await dialog.getByLabel("いまのパスワード").fill("not my password");
  await dialog.getByLabel(/確認のため/).fill(me.handle);
  await confirm.click();
  await expect(dialog.getByText("いまのパスワードがちがいます")).toBeVisible();

  await dialog.getByLabel("いまのパスワード").fill(PASSWORD);
  await confirm.click();
  const scheduled = page.getByTestId("deletion-scheduled");
  await expect(scheduled).toContainText("に削除されます");
  await expect(scheduled).toContainText(/あと(29|30)日/);
  expect((await getMe(request, me)).deletionScheduledAt).toBeTruthy();

  await page.getByRole("button", { name: "削除をキャンセルする" }).click();
  await expect(page.getByText("削除をキャンセルしました。おかえりなさい！")).toBeVisible();
  await expect(scheduled).toHaveCount(0);
  expect((await getMe(request, me)).deletionScheduledAt).toBeUndefined();
});

test("signing out of every device ends this session too", async ({ page, me: _me }) => {
  await page.goto("/settings/data");
  await page.getByRole("button", { name: "すべてのデバイスからログアウト" }).click();
  await page
    .getByRole("dialog", { name: "すべてのデバイスからログアウトする？" })
    .getByRole("button", { name: "すべてログアウト" })
    .click();
  await expect(page).toHaveURL(/\/$/);
  await page.goto("/settings");
  await expect(page).toHaveURL(/\/login\?redirect=/);
});
