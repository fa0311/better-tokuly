import { createHash, randomBytes } from "node:crypto";
import type { Page } from "@playwright/test";
import { createAccount, expect, rpc, test } from "./support";

const AUTH = "tokuly.identity.v1.AuthService";
const VERIFIER_HEADER = "x-tokuly-handoff-verifier";

interface Handoff {
  url: string;
}

/** A verifier the way the app makes one, and the hash it sends with the request. */
function verifier() {
  const value = randomBytes(32).toString("hex");
  return { value, hash: createHash("sha256").update(value).digest("hex") };
}

/** What the app's web view does: its first load of the hand-off carries the verifier. */
async function loadAsWebView(page: Page, url: string, value: string) {
  await page.route("**/api/auth/handoff*", (route) =>
    route.continue({ headers: { ...route.request().headers(), [VERIFIER_HEADER]: value } }),
  );
  await page.goto(url);
}

/**
 * The mobile app opens screens it does not have yet in a web view: it asks
 * for a one-time code as its own session (Bearer), bound to a verifier, and
 * loads the hand-off route, which signs the web view in and goes on to the
 * screen.
 */
test("an app's hand-off opens the web signed in, once", async ({ page, request }) => {
  const app = await createAccount(request, "handoff");
  const { value, hash } = verifier();
  const { url } = await rpc<Handoff>(
    request,
    `${AUTH}/CreateWebHandoff`,
    { path: "/settings", verifierHash: hash },
    app,
  );
  expect(url).toMatch(/^\/api\/auth\/handoff\?code=[\w-]{43,}$/);

  // A browser that never signed in (no hint of its own): the hand-off is enough.
  await loadAsWebView(page, url, value);
  await expect(page).toHaveURL(/\/settings$/);
  // The first load of a screen on a busy dev server takes a while.
  await expect(page.getByText(app.displayName).filter({ visible: true }).first()).toBeVisible({
    timeout: 60_000,
  });
  // The code is gone from the address, and nothing readable holds a token.
  expect(page.url()).not.toContain("code=");
  const readable = await page.evaluate(() => document.cookie);
  expect(readable).not.toContain("tokuly_rt");

  // Used: even with the verifier, another load lands on the sign-in page.
  const other = await page.context().browser()?.newContext();
  if (!other) throw new Error("no browser");
  const again = await other.newPage();
  await loadAsWebView(again, url, value);
  await expect(again).toHaveURL(/\/login/);
  await other.close();
});

/**
 * Login CSRF: a link with someone else's code, followed by a plain browser
 * (no verifier: a page cannot make a navigation carry the header), signs
 * nobody in, so the browser never ends up as the sender's account.
 */
test("a hand-off link followed by a browser signs nobody in", async ({ page, request }) => {
  const attacker = await createAccount(request, "attacker");
  const { url } = await rpc<Handoff>(
    request,
    `${AUTH}/CreateWebHandoff`,
    { path: "/map", verifierHash: verifier().hash },
    attacker,
  );
  await page.goto(url);
  await expect(page).toHaveURL(/\/login/);
  const signedIn = await page.request.post(`/api/${AUTH}/RefreshToken`, {
    data: {},
    headers: { "content-type": "application/json", "x-tokuly-client": "web" },
  });
  expect(signedIn.ok()).toBe(false);
});

test("a hand-off only leads to the web app's own screens", async ({ request }) => {
  const app = await createAccount(request, "redirect");
  for (const path of [
    "//evil.example",
    "https://evil.example/",
    "/\\evil.example",
    "/%2F%2Fevil",
  ]) {
    await expect(
      rpc(request, `${AUTH}/CreateWebHandoff`, { path, verifierHash: verifier().hash }, app),
      path,
    ).rejects.toMatchObject({ reason: "INVALID_PATH" });
  }
});
