import { createHash, createHmac, randomBytes } from "node:crypto";
import { createServer, type Server } from "node:http";
import type { AddressInfo } from "node:net";
import type { APIRequestContext, Page } from "@playwright/test";
import { type Account, createAccount, expect, PASSWORD, RpcError, rpc, test } from "./support";

const TOKENS = "tokuly.dev.v1.TokenService";
const APPS = "tokuly.dev.v1.AppService";
const BOTS = "tokuly.dev.v1.BotService";
const WEBHOOKS = "tokuly.dev.v1.WebhookService";

/** Calls an RPC with an API token instead of a session; returns the status and the error reason. */
async function callWithToken(
  request: APIRequestContext,
  procedure: string,
  token: string,
  body: object = {},
) {
  const response = await request.post(`/api/${procedure}`, {
    data: body,
    headers: { "content-type": "application/json", authorization: `Bearer ${token}` },
  });
  return {
    status: response.status(),
    reason: response.headers()["x-tokuly-error"] ?? "",
    scope: response.headers()["x-tokuly-scope"] ?? "",
    json: response.ok() ? ((await response.json()) as Record<string, unknown>) : {},
  };
}

/** The wording of a scope in the catalogue, as the screens show it (Japanese). */
async function scopeWording(request: APIRequestContext, scope: string): Promise<string> {
  const { scopes } = await rpc<{ scopes: { scope: string; description: { ja: string } }[] }>(
    request,
    `${TOKENS}/ListScopes`,
  );
  const found = scopes.find((entry) => entry.scope === scope);
  if (!found) throw new Error(`the scope catalogue has no ${scope}`);
  return found.description.ja;
}

/** Sets a domain of the scope picker to a level ("読む", "読む+書く"). */
async function chooseScope(page: Page, domain: string, level: string) {
  await page
    .getByRole("radiogroup", { name: domain, exact: true })
    .getByRole("radio", { name: level, exact: true })
    .click();
}

/** Reads the secret from the dialog that shows it once, then closes the dialog. */
async function takeSecret(page: Page): Promise<string> {
  const field = page.getByLabel("シークレット", { exact: true });
  await expect(field).toBeVisible();
  const secret = await field.inputValue();
  await page.getByRole("button", { name: "保存したので閉じる" }).click();
  await expect(field).toBeHidden();
  return secret;
}

/** Everything this origin keeps in the browser's storage, as one string. */
const browserStorage = (page: Page) =>
  page.evaluate(() => JSON.stringify([{ ...localStorage }, { ...sessionStorage }]));

/** A PKCE pair: the verifier the application keeps and the challenge it sends. */
function pkce() {
  const verifier = randomBytes(32).toString("base64url");
  return { verifier, challenge: createHash("sha256").update(verifier).digest("base64url") };
}

/** Where a test application's authorization code is sent; the browser never really goes there. */
const CALLBACK = "http://127.0.0.1:53999/callback";

/** Answers the application's callback address inside the browser, so no server is needed for it. */
async function interceptCallback(page: Page) {
  await page.route(`${CALLBACK}**`, (route) =>
    route.fulfill({ status: 200, contentType: "text/html", body: "<h1>callback</h1>" }),
  );
}

interface AppFixture {
  id: string;
  clientSecret: string;
}

/** Registers an OAuth application through the API. */
async function createApp(
  request: APIRequestContext,
  owner: Account,
  overrides: Record<string, unknown> = {},
): Promise<AppFixture & { name: string }> {
  const name = `E2E アプリ ${randomBytes(3).toString("hex")}`;
  const response = await rpc<{ app: { id: string }; clientSecret?: string }>(
    request,
    `${APPS}/CreateApp`,
    {
      name,
      description: "ブラウザテスト用のアプリ",
      redirectUris: [CALLBACK],
      scopes: ["identity:read", "notes:read"],
      clientType: "APP_CLIENT_TYPE_PUBLIC",
      ...overrides,
    },
    owner,
  );
  return { id: response.app.id, clientSecret: response.clientSecret ?? "", name };
}

/** The address an application sends the browser to. */
function authorizeUrl(clientId: string, challenge: string, extra: Record<string, string> = {}) {
  const query = new URLSearchParams({
    response_type: "code",
    client_id: clientId,
    redirect_uri: CALLBACK,
    scope: "identity:read",
    code_challenge: challenge,
    code_challenge_method: "S256",
    ...extra,
  });
  return `/oauth/authorize?${query}`;
}

/** Counts the calls that would approve something, so a test can show there were none. */
function watchApprovals(page: Page): { count(): number } {
  let approvals = 0;
  page.on("request", (request) => {
    if (/AuthorizationService\/Approve(Authorization|Device)/.test(request.url())) approvals += 1;
  });
  return { count: () => approvals };
}

test("a token is created, shown once, limited to its scopes, and stops when revoked", async ({
  page,
  request,
  me: _me,
}) => {
  await page.goto("/developers/tokens");
  await expect(page.getByText("トークンはまだありません")).toBeVisible();

  await page.getByRole("button", { name: "つくる", exact: true }).first().click();
  await page.getByLabel("なまえ").fill("テスト用スクリプト");
  // Location sits on its own, with a warning.
  await expect(page.getByText("ほんとうに必要なときだけオンにしてね")).toBeVisible();
  await chooseScope(page, "identity", "読む");
  await page.getByRole("button", { name: "トークンをつくる" }).click();

  await expect(page.getByText("表示されるのは今だけ!")).toBeVisible();
  const secret = await takeSecret(page);
  expect(secret).toMatch(/^tkl_pat_/);

  // Shown once: gone from the page, and never written to the browser's storage.
  await expect(page.getByText("テスト用スクリプト", { exact: true })).toBeVisible();
  expect(await page.content()).not.toContain(secret);
  expect(await browserStorage(page)).not.toContain(secret);
  await expect(page.getByText("identity:read", { exact: true })).toBeVisible();

  // The token works for what it was given...
  const whoAmI = await callWithToken(request, `${TOKENS}/GetCurrentToken`, secret);
  expect(whoAmI.status).toBe(200);
  expect(whoAmI.json.kind).toBe("CREDENTIAL_KIND_PERSONAL_TOKEN");
  expect(whoAmI.json.scopes).toEqual(["identity:read"]);
  const profile = await callWithToken(request, "tokuly.identity.v1.AccountService/GetMe", secret);
  expect(profile.status).toBe(200);
  // ...and is refused where it has no scope.
  const balance = await callWithToken(request, "tokuly.wallet.v1.WalletService/GetBalance", secret);
  expect(balance.status).toBe(403);
  expect(balance.reason).toBe("INSUFFICIENT_SCOPE");
  expect(balance.scope).toContain("wallet:read");

  await page.getByRole("button", { name: "無効にする" }).click();
  await page.getByRole("dialog").getByRole("button", { name: "無効にする" }).click();
  await expect(page.getByText("トークンを無効にしました")).toBeVisible();
  await expect(page.getByText("トークンはまだありません")).toBeVisible();
  const afterRevoke = await callWithToken(request, `${TOKENS}/GetCurrentToken`, secret);
  expect(afterRevoke.status).toBe(401);
});

test("an OAuth app is registered and the PKCE flow leads through the consent screen to a token", async ({
  page,
  request,
  me,
}) => {
  // Register a confidential application in the UI: its secret is shown once.
  await page.goto("/developers/apps");
  await page.getByRole("button", { name: "登録する", exact: true }).first().click();
  await page.getByLabel("アプリ名").fill("おでかけログ");
  await page.getByLabel("リダイレクトURI").fill(CALLBACK);
  await chooseScope(page, "identity", "読む");
  await page.getByRole("dialog").getByRole("button", { name: "登録する" }).click();
  const clientSecret = await takeSecret(page);
  expect(clientSecret).toMatch(/^tkl_cs_/);

  await expect(page).toHaveURL(/\/developers\/apps\/[0-9a-f-]+$/);
  const clientId = new URL(page.url()).pathname.split("/").pop() ?? "";
  await expect(page.getByText(clientId, { exact: true })).toBeVisible();
  expect(await page.content()).not.toContain(clientSecret);
  expect(await browserStorage(page)).not.toContain(clientSecret);

  // The application sends the browser to the consent page. A state that looks like a number
  // must come back exactly as it was sent.
  const { verifier, challenge } = pkce();
  const state = "1234e56";
  await interceptCallback(page);
  const approvals = watchApprovals(page);
  const response = await page.goto(authorizeUrl(clientId, challenge, { state }));
  // The page may not be framed by anyone.
  expect(response?.headers()["x-frame-options"]).toBe("DENY");
  expect(response?.headers()["content-security-policy"]).toContain("frame-ancestors 'none'");

  // Who is asking, who made it, and every scope in the catalogue's words.
  await expect(page.getByRole("heading", { name: "アプリと連携する?" })).toBeVisible();
  await expect(page.getByText("おでかけログ")).toBeVisible();
  await expect(page.getByText(`@${me.handle}`, { exact: true })).toBeVisible();
  await expect(page.getByText(await scopeWording(request, "identity:read"))).toBeVisible();
  await expect(page.getByText("127.0.0.1 に移動します")).toBeVisible();
  expect(approvals.count()).toBe(0);

  await page.getByRole("button", { name: "許可する", exact: true }).click();
  await page.waitForURL(`${CALLBACK}**`);
  expect(approvals.count()).toBe(1);
  const callback = new URL(page.url());
  expect(callback.searchParams.get("state")).toBe(state);
  const code = callback.searchParams.get("code") ?? "";
  expect(code).not.toBe("");

  // The application exchanges the code for tokens and uses them.
  const tokens = await request.post("/api/oauth/token", {
    form: {
      grant_type: "authorization_code",
      code,
      code_verifier: verifier,
      redirect_uri: CALLBACK,
      client_id: clientId,
      client_secret: clientSecret,
    },
  });
  expect(tokens.ok(), await tokens.text()).toBeTruthy();
  const { access_token: accessToken } = (await tokens.json()) as { access_token: string };
  const whoAmI = await callWithToken(request, `${TOKENS}/GetCurrentToken`, accessToken);
  expect(whoAmI.json.kind).toBe("CREDENTIAL_KIND_APP");
  expect(whoAmI.json.scopes).toEqual(["identity:read"]);

  // The person finds the application among those with access and withdraws it.
  await page.goto("/developers/authorized");
  await expect(page.getByText("おでかけログ")).toBeVisible();
  await page.getByRole("button", { name: "連携を解除" }).click();
  await page.getByRole("dialog").getByRole("button", { name: "連携を解除" }).click();
  await expect(page.getByText("連携しているアプリはありません")).toBeVisible();
  expect((await callWithToken(request, `${TOKENS}/GetCurrentToken`, accessToken)).status).toBe(401);
});

test("the consent page never approves by itself and only leaves when a button is pressed", async ({
  page,
  request,
  me,
}) => {
  const app = await createApp(request, me);
  const { challenge } = pkce();
  await interceptCallback(page);
  const approvals = watchApprovals(page);

  // Parameters that try to skip the screen change nothing.
  await page.goto(
    authorizeUrl(app.id, challenge, {
      state: "s1",
      approve: "1",
      auto: "true",
      confirm: "yes",
      prompt: "none",
    }),
  );
  await expect(page.getByRole("button", { name: "許可する", exact: true })).toBeVisible();
  await page.waitForTimeout(1500);
  expect(approvals.count()).toBe(0);
  expect(new URL(page.url()).pathname).toBe("/oauth/authorize");

  // Refusing tells the application so, and nothing was approved.
  await page.getByRole("button", { name: "許可しない" }).click();
  await page.waitForURL(`${CALLBACK}**`);
  const denied = new URL(page.url());
  expect(denied.searchParams.get("error")).toBe("access_denied");
  expect(denied.searchParams.get("state")).toBe("s1");
  expect(denied.searchParams.get("code")).toBeNull();
  expect(approvals.count()).toBe(0);

  // Having agreed before does not skip the screen either: the person presses the button again.
  await page.goto(authorizeUrl(app.id, challenge, { state: "s2" }));
  await page.getByRole("button", { name: "許可する", exact: true }).click();
  await page.waitForURL(`${CALLBACK}**`);
  await page.goto(authorizeUrl(app.id, challenge, { state: "s3" }));
  await expect(page.getByText("前にも同じ内容を許可しています")).toBeVisible();
  await page.waitForTimeout(1000);
  expect(new URL(page.url()).pathname).toBe("/oauth/authorize");
  expect(approvals.count()).toBe(1);
});

test("a redirect address the app never registered is shown as an error, not followed", async ({
  page,
  request,
  me,
}) => {
  const app = await createApp(request, me);
  const { challenge } = pkce();
  let leftForElsewhere = false;
  await page.route("https://evil.example/**", (route) => {
    leftForElsewhere = true;
    return route.fulfill({ status: 200, body: "evil" });
  });

  await page.goto(
    authorizeUrl(app.id, challenge, { redirect_uri: "https://evil.example/steal", state: "x" }),
  );
  await expect(
    page.getByText("もどり先のアドレスが、アプリに登録されたものとちがいます"),
  ).toBeVisible();
  await expect(page.getByRole("button", { name: "許可する", exact: true })).toHaveCount(0);
  expect(new URL(page.url()).pathname).toBe("/oauth/authorize");
  expect(leftForElsewhere).toBe(false);

  // An application that does not exist is an error on the page as well.
  await page.goto(authorizeUrl("00000000-0000-7000-8000-000000000000", challenge));
  await expect(page.getByText("このアプリは見つからないか、いまは使えません")).toBeVisible();
});

test("a visitor signs in first and comes back to the same consent request", async ({
  browser,
  request,
  baseURL,
}) => {
  const owner = await createAccount(request, "owner");
  const visitor = await createAccount(request, "visitor");
  const app = await createApp(request, owner);
  const { challenge } = pkce();

  const context = await browser.newContext({ baseURL });
  const page = await context.newPage();
  await interceptCallback(page);
  const approvals = watchApprovals(page);
  const state = "9e9";
  await page.goto(authorizeUrl(app.id, challenge, { state }));
  await expect(page.getByRole("heading", { name: "ログインして続ける" })).toBeVisible();
  await page.getByRole("link", { name: "ログイン", exact: true }).click();

  await page.getByLabel("メールアドレス または ハンドル").fill(visitor.handle);
  await page.getByLabel("パスワード").fill(PASSWORD);
  await page.getByRole("button", { name: "ログイン", exact: true }).click();

  // Back on the consent page, with the request intact; signing in approved nothing.
  await expect(page.getByRole("heading", { name: "アプリと連携する?" })).toBeVisible();
  await expect(page.getByText(app.name)).toBeVisible();
  await expect(page.getByText(`@${owner.handle}`, { exact: true })).toBeVisible();
  await expect(page.getByText(`@${visitor.handle} としてログイン中`)).toBeVisible();
  expect(approvals.count()).toBe(0);

  await page.getByRole("button", { name: "許可する", exact: true }).click();
  await page.waitForURL(`${CALLBACK}**`);
  expect(new URL(page.url()).searchParams.get("state")).toBe(state);
  await context.close();
});

test("the device flow: the code is typed, checked against the device, and approved", async ({
  page,
  request,
  me,
}) => {
  const app = await createApp(request, me, { deviceFlow: true });
  const started = await request.post("/api/oauth/device_authorization", {
    form: { client_id: app.id, scope: "identity:read" },
  });
  expect(started.ok(), await started.text()).toBeTruthy();
  const device = (await started.json()) as { device_code: string; user_code: string };
  const poll = () =>
    request.post("/api/oauth/token", {
      form: {
        grant_type: "urn:ietf:params:oauth:grant-type:device_code",
        device_code: device.device_code,
        client_id: app.id,
      },
    });
  expect(((await (await poll()).json()) as { error: string }).error).toBe("authorization_pending");

  const approvals = watchApprovals(page);
  const response = await page.goto(`/oauth/device?user_code=${device.user_code}`);
  expect(response?.headers()["x-frame-options"]).toBe("DENY");
  // The code from the address only fills the field: nothing is looked up or approved yet.
  await expect(page.getByLabel("コード")).toHaveValue(device.user_code);
  await page.getByRole("button", { name: "確認する" }).click();

  await expect(page.getByText(app.name)).toBeVisible();
  await expect(
    page.getByText("だれかに送られてきたコードは、ぜったいに許可しないで!"),
  ).toBeVisible();
  await expect(page.getByText(await scopeWording(request, "identity:read"))).toBeVisible();
  expect(approvals.count()).toBe(0);
  await page.getByRole("button", { name: "許可する", exact: true }).click();
  await expect(page.getByText("連携できました!")).toBeVisible();

  // The waiting device gets its token (it may be told to slow down once first).
  let accessToken = "";
  await expect(async () => {
    const answer = (await (await poll()).json()) as { access_token?: string };
    expect(answer.access_token).toBeTruthy();
    accessToken = answer.access_token ?? "";
  }).toPass({ timeout: 30_000, intervals: [6_000] });
  const whoAmI = await callWithToken(request, `${TOKENS}/GetCurrentToken`, accessToken);
  expect(whoAmI.json.kind).toBe("CREDENTIAL_KIND_APP");

  // A code nobody issued is refused.
  await page.goto("/oauth/device");
  await page.getByLabel("コード").fill("ZZZZ-ZZZZ");
  await page.getByRole("button", { name: "確認する" }).click();
  await expect(page.getByText("このコードは無効か、期限が切れています")).toBeVisible();
});

test("a bot gets an account and a read-only token of its own, and can be switched off", async ({
  page,
  request,
  me,
}) => {
  const handle = `bot_${randomBytes(4).toString("hex")}`;
  await page.goto("/developers/bots");
  await page.getByRole("button", { name: "つくる", exact: true }).first().click();
  await page.getByLabel("ハンドル").fill(handle);
  await page.getByLabel("なまえ").fill("おしらせBot");
  await page.getByRole("button", { name: "Botをつくる" }).click();

  await expect(page).toHaveURL(/\/developers\/bots\/[0-9a-f-]+$/);
  await expect(page.getByRole("link", { name: `@${handle}` })).toBeVisible();

  // A bot's token can read, never write, and never sees the wallet.
  await page.getByRole("button", { name: "トークンをつくる" }).click();
  const form = page.getByRole("dialog");
  await form.getByLabel("なまえ").fill("本番");
  await expect(form.getByRole("radiogroup", { name: "notes", exact: true })).toBeVisible();
  await expect(form.getByRole("radio", { name: "読む+書く" })).toHaveCount(0);
  await expect(form.getByRole("radiogroup", { name: "wallet", exact: true })).toHaveCount(0);
  await chooseScope(page, "notes", "読む");
  await form.getByRole("button", { name: "トークンをつくる" }).click();
  const secret = await takeSecret(page);
  expect(secret).toMatch(/^tkl_bot_/);
  expect(await browserStorage(page)).not.toContain(secret);

  const whoAmI = await callWithToken(request, `${TOKENS}/GetCurrentToken`, secret);
  expect(whoAmI.json.kind).toBe("CREDENTIAL_KIND_BOT");
  expect((whoAmI.json.user as { handle: string }).handle).toBe(handle);

  // Switched off, its token is refused; switched on again, it works.
  await page.getByRole("switch").click();
  await expect(page.getByText("オフ", { exact: true })).toBeVisible();
  expect((await callWithToken(request, `${TOKENS}/GetCurrentToken`, secret)).status).not.toBe(200);
  await page.getByRole("switch").click();
  await expect(page.getByText("オフ", { exact: true })).toBeHidden();
  expect((await callWithToken(request, `${TOKENS}/GetCurrentToken`, secret)).status).toBe(200);

  // Deleting the bot takes its tokens with it.
  await page.getByRole("button", { name: "削除する" }).click();
  await page.getByRole("dialog").getByRole("button", { name: "削除する" }).click();
  await expect(page).toHaveURL(/\/developers\/bots$/);
  await expect(page.getByText("Botはまだいません")).toBeVisible();
  expect((await callWithToken(request, `${TOKENS}/GetCurrentToken`, secret)).status).not.toBe(200);
  const { bots } = await rpc<{ bots?: unknown[] }>(request, `${BOTS}/ListBots`, {}, me);
  expect(bots ?? []).toHaveLength(0);
});

test("a webhook is added, a test delivery shows in the log with its payload, and replay sends it again", async ({
  page,
  me: _me,
}) => {
  await page.goto("/developers/webhooks");
  await page.getByRole("button", { name: "追加", exact: true }).first().click();
  // A name that does not resolve: the delivery fails, which the log must show just as well.
  await page.getByLabel("送り先のURL").fill("https://hooks.tokuly-e2e.example.com/in");
  await page.getByLabel("メモ").fill("本番サーバー");
  await page.getByRole("checkbox", { name: /friend\.added/ }).check();
  await page.getByRole("dialog").getByRole("button", { name: "追加する" }).click();
  const secret = await takeSecret(page);
  expect(secret).toMatch(/^tkl_whs_/);
  expect(await browserStorage(page)).not.toContain(secret);

  await expect(page).toHaveURL(/\/developers\/webhooks\/[0-9a-f-]+$/);
  await expect(page.getByText("受信中")).toBeVisible();
  await expect(page.getByText("まだなにも送られていません")).toBeVisible();

  // A test delivery: the dialog shows what was sent and how the endpoint answered.
  await page.getByRole("button", { name: "テスト送信" }).click();
  const detail = page.getByRole("dialog");
  await expect(detail.getByRole("heading", { name: "配信の詳細" })).toBeVisible();
  await expect(detail.getByText('"type": "ping"')).toBeVisible();
  await expect(detail.getByText("試行(1回)")).toBeVisible();
  await expect(detail.getByText("失敗", { exact: true })).toBeVisible();

  // Replay sends the same event again as a new delivery.
  await detail.getByRole("button", { name: "もう一度送る" }).click();
  await expect(page.getByText("もう一度送りました!")).toBeVisible();
  await expect(detail.getByText("再送", { exact: true })).toBeVisible();
  await detail.getByRole("button", { name: "close" }).click();

  const log = page.getByRole("list", { name: "配信ログ" });
  await expect(log.getByRole("listitem")).toHaveCount(2);
  await expect(log.getByText("再送")).toHaveCount(1);
  // The log filters by outcome.
  await page.getByRole("tab", { name: "成功" }).click();
  await expect(page.getByText("まだなにも送られていません")).toBeVisible();
  await page.getByRole("tab", { name: "失敗" }).click();
  await expect(log.getByRole("listitem")).toHaveCount(2);

  // Pausing stops deliveries; a paused webhook cannot be replayed to.
  await page.getByRole("switch").click();
  await expect(page.getByText("一時停止", { exact: true })).toBeVisible();
});

test("a webhook receives a signed delivery, the log shows it, and replay delivers it again", async ({
  page,
  request,
  me,
}) => {
  // A tiny receiver inside the test. The API must be able to reach this machine's loopback
  // address for it, which only a development API allows.
  const received: { signature: string; event: string; body: string }[] = [];
  const server: Server = createServer((incoming, outgoing) => {
    let body = "";
    incoming.on("data", (chunk) => {
      body += chunk;
    });
    incoming.on("end", () => {
      received.push({
        signature: String(incoming.headers["tokuly-signature"] ?? ""),
        event: String(incoming.headers["tokuly-event"] ?? ""),
        body,
      });
      outgoing.writeHead(200, { "content-type": "text/plain" }).end("thanks");
    });
  });
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  const { port } = server.address() as AddressInfo;

  try {
    let webhook: { webhook: { id: string }; secret: string };
    try {
      webhook = await rpc(
        request,
        `${WEBHOOKS}/CreateWebhook`,
        { url: `http://127.0.0.1:${port}/hook`, eventTypes: ["friend.added"] },
        me,
      );
    } catch (error) {
      test.skip(
        error instanceof RpcError && error.reason === "WEBHOOK_URL_INVALID",
        "this API only delivers to public https addresses (TOKULY_DEV_WEBHOOK_ALLOW_PRIVATE is off), so a receiver inside the test cannot be reached",
      );
      throw error;
    }

    await page.goto(`/developers/webhooks/${webhook.webhook.id}`);
    await page.getByRole("button", { name: "テスト送信" }).click();
    const detail = page.getByRole("dialog");
    await expect(detail.getByText("成功", { exact: true })).toBeVisible();
    await expect(detail.getByText("HTTP 200")).toBeVisible();
    await expect(detail.getByText("thanks")).toBeVisible();

    // What arrived is signed with the webhook's secret: v1 = HMAC-SHA256(secret, "<t>.<body>").
    expect(received).toHaveLength(1);
    const first = received[0];
    if (!first) throw new Error("no delivery arrived");
    expect(first.event).toBe("ping");
    const parts = new Map(
      first.signature.split(",").map((part) => part.split("=") as [string, string]),
    );
    const expected = createHmac("sha256", webhook.secret)
      .update(`${parts.get("t")}.${first.body}`)
      .digest("hex");
    expect(parts.get("v1")).toBe(expected);

    await detail.getByRole("button", { name: "もう一度送る" }).click();
    await expect(page.getByText("もう一度送りました!")).toBeVisible();
    await expect.poll(() => received.length).toBe(2);
    expect(JSON.parse(received[1]?.body ?? "{}").id).toBe(JSON.parse(first.body).id);
  } finally {
    server.close();
  }
});

test("the API reference lists the API, links deep, and its try-it panel returns a real answer", async ({
  page,
  request,
  me,
}) => {
  const { secret } = await rpc<{ secret: string }>(
    request,
    `${TOKENS}/CreateToken`,
    { name: "reference", scopes: ["identity:read"] },
    me,
  );

  // A deep link opens a method with what it needs and how to call it.
  await page.goto("/docs/api/tokuly.notes.v1.NoteService/CreateNote");
  await expect(page.getByRole("heading", { name: "CreateNote", level: 1 })).toBeVisible();
  await expect(page.getByRole("main").getByText("notes:write").first()).toBeVisible();
  await expect(page.getByText(await scopeWording(request, "notes:write"))).toBeVisible();
  await expect(
    page.getByText("/tokuly.notes.v1.NoteService/CreateNote", { exact: true }),
  ).toBeVisible();
  await expect(
    page.getByText(/curl http.*\/api\/tokuly\.notes\.v1\.NoteService\/CreateNote/),
  ).toBeVisible();

  // Search finds a method by name.
  await page.goto("/docs/api");
  const services = page.getByRole("navigation", { name: "サービス一覧" }).filter({ visible: true });
  await expect(services.getByText("NoteService", { exact: true })).toBeVisible();
  await services.getByRole("searchbox", { name: "メソッドをさがす" }).fill("GetCurrentToken");
  await services.getByRole("link", { name: /GetCurrentToken/ }).click();
  await expect(page).toHaveURL(/\/docs\/api\/tokuly\.dev\.v1\.TokenService\/GetCurrentToken$/);
  await expect(page.getByRole("heading", { name: "GetCurrentToken", level: 1 })).toBeVisible();
  await expect(page.getByText("どのトークンでもOK")).toBeVisible();

  // Try it: a real call with the token, answered by the real API.
  const panel = page.getByRole("region", { name: "ためす" });
  await panel.getByLabel("トークン").fill(secret);
  await panel.getByRole("button", { name: "送信" }).click();
  const answer = panel.getByRole("region", { name: "レスポンス" });
  await expect(answer.getByText("200")).toBeVisible();
  await expect(answer.getByText('"CREDENTIAL_KIND_PERSONAL_TOKEN"')).toBeVisible();
  await expect(answer.getByText(`"${me.handle}"`)).toBeVisible();
  // The pasted token stays in memory only.
  expect(await browserStorage(page)).not.toContain(secret);

  // Without a token the same call is refused, and the panel shows why.
  await panel.getByLabel("トークン").fill("");
  await panel.getByRole("button", { name: "送信" }).click();
  await expect(answer.getByText("401")).toBeVisible();
});

test("the hub and the settings lead to the developer pages", async ({ page, me: _me }) => {
  await page.goto("/me");
  await expect(page.getByRole("main").getByRole("link", { name: "デベロッパー" })).toHaveAttribute(
    "href",
    "/developers",
  );
  await page.goto("/developers");
  for (const [label, path] of [
    ["アクセストークン", "/developers/tokens"],
    ["OAuthアプリ", "/developers/apps"],
    ["Bot", "/developers/bots"],
    ["Webhook", "/developers/webhooks"],
    ["連携アプリ", "/developers/authorized"],
    ["APIリファレンス", "/docs/api"],
  ] as const) {
    await expect(page.getByRole("main").getByRole("link", { name: label })).toHaveAttribute(
      "href",
      path,
    );
  }
  await page.goto("/settings");
  await expect(page.getByRole("link", { name: /連携アプリ/ }).first()).toHaveAttribute(
    "href",
    "/developers/authorized",
  );
});
