import {
  type APIRequestContext,
  type BrowserContext,
  test as base,
  expect,
  type Page,
} from "@playwright/test";

export { expect };

/** Every test account uses this password. */
export const PASSWORD = "correct horse battery";

/**
 * Address of the account the server promotes to admin once it is verified
 * (`TOKULY_ADMIN_USER` of the server under test).
 */
const ADMIN_EMAIL = process.env.TOKULY_E2E_ADMIN_EMAIL ?? "e2e-admin@e2e.tokuly.test";
const ADMIN_HANDLE = "e2e_admin";

/** An account created for a test, with a token for calling the API as it. */
export interface Account {
  userId: string;
  handle: string;
  displayName: string;
  email: string;
  accessToken: string;
  roles: string[];
}

/** An API failure: `reason` is the server's error reason (`HANDLE_TAKEN`, ...). */
export class RpcError extends Error {
  constructor(
    readonly procedure: string,
    readonly status: number,
    readonly reason: string,
    detail: string,
  ) {
    super(`${procedure} failed (${status} ${reason}): ${detail}`);
  }
}

/**
 * Calls an RPC with the JSON Connect protocol, as `as` when given. Use it to
 * arrange the state a test needs (friends, messages, purchases) and to check
 * what the UI did. `procedure` is `package.Service/Method`; request and
 * response use the proto JSON field names (camelCase).
 */
export async function rpc<T = Record<string, unknown>>(
  request: APIRequestContext,
  procedure: string,
  body: object = {},
  as?: Account,
): Promise<T> {
  const response = await request.post(`/api/${procedure}`, {
    data: body,
    headers: {
      "content-type": "application/json",
      ...(as ? { authorization: `Bearer ${as.accessToken}` } : {}),
    },
  });
  if (!response.ok()) {
    throw new RpcError(
      procedure,
      response.status(),
      response.headers()["x-tokuly-error"] ?? "",
      (await response.text()).slice(0, 300),
    );
  }
  return (await response.json()) as T;
}

interface AuthResponse {
  accessToken: string;
  me: {
    user: { id: string; handle: string; displayName: string };
    email: string;
    roles?: string[];
  };
}

function toAccount(auth: AuthResponse): Account {
  return {
    userId: auth.me.user.id,
    handle: auth.me.user.handle,
    displayName: auth.me.user.displayName,
    email: auth.me.email,
    accessToken: auth.accessToken,
    roles: auth.me.roles ?? [],
  };
}

function uniqueSuffix(): string {
  return `${Date.now().toString(36).slice(-5)}${Math.random().toString(36).slice(2, 6)}`;
}

/**
 * Signs up a fresh account. `name` becomes the display name and the start of
 * the handle; `birthdate` (YYYY-MM-DD) defaults to an adult's.
 */
export async function createAccount(
  request: APIRequestContext,
  name: string,
  { birthdate = "2000-04-02" }: { birthdate?: string } = {},
): Promise<Account> {
  const handle = `${name
    .toLowerCase()
    .replace(/[^a-z0-9_]/g, "")
    .slice(0, 12)}_${uniqueSuffix()}`;
  const auth = await rpc<AuthResponse>(request, "tokuly.identity.v1.AuthService/SignUp", {
    email: `${handle}@e2e.tokuly.test`,
    password: PASSWORD,
    handle,
    displayName: name,
    birthdate,
    acceptTerms: true,
  });
  return toAccount(auth);
}

/** A fresh access token for an account (roles and other claims are re-read). */
export async function refreshAccount(
  request: APIRequestContext,
  account: Account,
): Promise<Account> {
  return toAccount(
    await rpc<AuthResponse>(request, "tokuly.identity.v1.AuthService/SignIn", {
      identifier: account.handle,
      password: PASSWORD,
    }),
  );
}

/**
 * The deployment's admin account. The server grants the role once the
 * address named by `TOKULY_ADMIN_USER` is verified, so this signs up with it,
 * opens the verification link from Mailpit and waits until a sign-in
 * carries the role.
 */
export async function adminAccount(request: APIRequestContext): Promise<Account> {
  try {
    await rpc(request, "tokuly.identity.v1.AuthService/SignUp", {
      email: ADMIN_EMAIL,
      password: PASSWORD,
      handle: ADMIN_HANDLE,
      displayName: "Admin",
      birthdate: "1990-01-01",
      acceptTerms: true,
    });
  } catch (error) {
    // Another test (or an earlier run) created it already.
    if (!(error instanceof RpcError) || !/TAKEN|EXISTS/.test(error.reason)) throw error;
  }
  const signIn = async () =>
    toAccount(
      await rpc<AuthResponse>(request, "tokuly.identity.v1.AuthService/SignIn", {
        identifier: ADMIN_EMAIL,
        password: PASSWORD,
      }),
    );
  let account = await signIn();
  if (!account.roles.includes("admin")) {
    const { me } = await rpc<{ me: { emailVerified?: boolean } }>(
      request,
      "tokuly.identity.v1.AccountService/GetMe",
      {},
      account,
    );
    if (!me.emailVerified) {
      // Tests running at once may race to open the link; one of them wins.
      await rpc(request, "tokuly.identity.v1.AuthService/VerifyEmail", {
        token: await mailedVerificationToken(request, ADMIN_EMAIL),
      }).catch((error) => {
        if (!(error instanceof RpcError) || error.reason !== "TOKEN_INVALID") throw error;
      });
    }
  }
  await expect(async () => {
    account = await signIn();
    expect(account.roles).toContain("admin");
  }).toPass({ timeout: 20_000 });
  return account;
}

/** Where the development stack delivers mail (Mailpit); tests that read mail skip without it. */
const MAILPIT = process.env.TOKULY_MAILPIT_URL ?? "http://127.0.0.1:58025";

export async function mailpitIsUp(request: APIRequestContext): Promise<boolean> {
  try {
    return (await request.get(`${MAILPIT}/api/v1/info`, { timeout: 3000 })).ok();
  } catch {
    return false;
  }
}

/**
 * The token of the newest link to `path` (a verification link unless told
 * otherwise) mailed to `to`, in a mail with `subject` when given.
 */
export async function mailedVerificationToken(
  request: APIRequestContext,
  to: string,
  subject?: string,
  path = "verify-email",
): Promise<string> {
  let token = "";
  const query = subject ? `to:${to} subject:"${subject}"` : `to:${to}`;
  await expect(async () => {
    const found = (await (
      await request.get(`${MAILPIT}/api/v1/search`, { params: { query, limit: 5 } })
    ).json()) as { messages: { ID: string }[] };
    for (const { ID } of found.messages) {
      const message = (await (await request.get(`${MAILPIT}/api/v1/message/${ID}`)).json()) as {
        Text?: string;
      };
      const link = new RegExp(`/${path}\\?token=([\\w.~%-]+)`).exec(message.Text ?? "");
      if (link?.[1]) {
        token = decodeURIComponent(link[1]);
        return;
      }
    }
    expect(token, `a verification mail to ${to}`).not.toBe("");
  }).toPass({ timeout: 20_000 });
  return token;
}

/** Makes two accounts friends (each requests the other; the second request accepts the first). */
export async function befriend(request: APIRequestContext, a: Account, b: Account): Promise<void> {
  await rpc(request, "tokuly.social.v1.FriendService/SendFriendRequest", { userId: b.userId }, a);
  await rpc(request, "tokuly.social.v1.FriendService/SendFriendRequest", { userId: a.userId }, b);
}

/**
 * Signs the browser in as `account` without going through the login form:
 * the web sign-in call leaves the refresh cookie in the browser context, and
 * the app restores the session from it on the next page load.
 */
export async function signInAs(page: Page, account: Account): Promise<void> {
  const response = await page.request.post("/api/tokuly.identity.v1.AuthService/SignIn", {
    data: { identifier: account.handle, password: PASSWORD },
    headers: { "content-type": "application/json", "x-tokuly-client": "web" },
  });
  expect(response.ok(), `sign-in as ${account.handle}`).toBeTruthy();
  await page.addInitScript(() => window.localStorage.setItem("tokuly.signedIn", "1"));
}

/**
 * Lets a toast that landed under the pointer leave again. Toasts from any
 * feature can appear at any moment (a badge for the first friend, a message
 * from another test's account), and one may slide in right where a test is
 * about to press. Playwright then keeps its pointer resting on the toast while
 * it retries, and sonner keeps a hovered toast on screen (so that it can be
 * read), so it never leaves and the press times out. A person would look
 * elsewhere: here the pointer moves off the toast and the toast times out.
 * Pressing a toast's own buttons still works: the press moves the pointer back.
 */
export async function letHoveredToastsGo(page: Page): Promise<void> {
  await page.addLocatorHandler(
    page.locator("[data-sonner-toast]:hover"),
    async () => {
      await page.mouse.move(0, 0);
    },
    { noWaitAfter: true },
  );
}

function guardPages(context: BrowserContext): void {
  context.on("page", (page) => void letHoveredToastsGo(page));
}

/**
 * `test` with a signed-in browser: `me` is a fresh account and `page` is
 * already signed in as it. Every page it opens, in its own context or in one
 * a test makes with `browser.newContext()`, lets hovered toasts go (see
 * `letHoveredToastsGo`).
 */
export const test = base.extend<{ me: Account }>({
  context: async ({ context }, use) => {
    guardPages(context);
    await use(context);
  },
  browser: [
    async ({ browser }, use) => {
      const newContext = browser.newContext.bind(browser);
      browser.newContext = async (options) => {
        const context = await newContext(options);
        guardPages(context);
        return context;
      };
      await use(browser);
    },
    { scope: "worker" },
  ],
  me: async ({ page, request }, use) => {
    const account = await createAccount(request, "me");
    await signInAs(page, account);
    await use(account);
  },
});
