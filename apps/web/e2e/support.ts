import { type APIRequestContext, test as base, expect, type Page } from "@playwright/test";

export { expect };

/** Every test account uses this password. */
export const PASSWORD = "correct horse battery";

/** Handle of the account the server promotes to admin (`TOKULY_ADMIN_USER`). */
const ADMIN_HANDLE = process.env.TOKULY_E2E_ADMIN ?? "tokuly_admin";

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

/** Signs up a fresh account. `name` becomes the display name and the start of the handle. */
export async function createAccount(request: APIRequestContext, name: string): Promise<Account> {
  const handle = `${name
    .toLowerCase()
    .replace(/[^a-z0-9_]/g, "")
    .slice(0, 12)}_${uniqueSuffix()}`;
  const auth = await rpc<AuthResponse>(request, "tokuly.identity.v1.AuthService/SignUp", {
    email: `${handle}@e2e.tokuly.test`,
    password: PASSWORD,
    handle,
    displayName: name,
    birthdate: "2000-04-02",
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
 * The deployment's admin account. The server grants the role when the
 * account named by `TOKULY_ADMIN_USER` signs up, a moment after the fact, so
 * this waits until a sign-in carries it.
 */
export async function adminAccount(request: APIRequestContext): Promise<Account> {
  try {
    await rpc(request, "tokuly.identity.v1.AuthService/SignUp", {
      email: `${ADMIN_HANDLE}@e2e.tokuly.test`,
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
  let account: Account | undefined;
  await expect(async () => {
    account = toAccount(
      await rpc<AuthResponse>(request, "tokuly.identity.v1.AuthService/SignIn", {
        identifier: ADMIN_HANDLE,
        password: PASSWORD,
      }),
    );
    expect(account.roles).toContain("admin");
  }).toPass({ timeout: 20_000 });
  return account as Account;
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
 * `test` with a signed-in browser: `me` is a fresh account and `page` is
 * already signed in as it.
 */
export const test = base.extend<{ me: Account }>({
  me: async ({ page, request }, use) => {
    const account = await createAccount(request, "me");
    await signInAs(page, account);
    await use(account);
  },
});
