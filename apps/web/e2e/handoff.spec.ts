import { createAccount, expect, rpc, test } from "./support";

const AUTH = "tokuly.identity.v1.AuthService";

interface Handoff {
  url: string;
}

/**
 * The mobile app opens screens it does not have yet in a web view: it asks
 * for a one-time code as its own session (Bearer) and loads the hand-off
 * route, which signs the web view in and goes on to the screen.
 */
test("an app's hand-off opens the web signed in, once", async ({ page, request }) => {
  const app = await createAccount(request, "handoff");
  const { url } = await rpc<Handoff>(
    request,
    `${AUTH}/CreateWebHandoff`,
    { path: "/settings" },
    app,
  );
  expect(url).toMatch(/^\/api\/auth\/handoff\?code=[\w-]{43,}$/);

  // A browser that never signed in (no hint of its own): the hand-off is enough.
  await page.goto(url);
  await expect(page).toHaveURL(/\/settings$/);
  await expect(page.getByText(app.displayName).filter({ visible: true }).first()).toBeVisible({
    timeout: 25_000,
  });
  // The code is gone from the address, and nothing readable holds a token.
  expect(page.url()).not.toContain("code=");
  const readable = await page.evaluate(() => document.cookie);
  expect(readable).not.toContain("tokuly_rt");

  // Used: another browser loading it lands on the sign-in page.
  const other = await page.context().browser()?.newContext();
  if (!other) throw new Error("no browser");
  const stranger = await other.newPage();
  await stranger.goto(url);
  await expect(stranger).toHaveURL(/\/login/);
  await other.close();
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
      rpc(request, `${AUTH}/CreateWebHandoff`, { path }, app),
      path,
    ).rejects.toMatchObject({ reason: "INVALID_PATH" });
  }
});
