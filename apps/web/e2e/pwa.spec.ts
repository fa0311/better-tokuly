import { type APIRequestContext, test as base, type Page } from "@playwright/test";
import { expect as baseExpect, createAccount, rpc, test } from "./support";

// These tests share a machine with many other builds and browsers: a cold page
// or a busy API can take a while to answer. Waiting longer costs nothing when
// things are quick.
const expect = baseExpect.configure({ timeout: 20_000 });
test.describe.configure({ timeout: 90_000 });

declare global {
  interface Window {
    /** How often the (stand-in) install dialog of the browser was opened. */
    __installPrompts?: number;
    /** What was put on the app's icon, in order (0 = cleared). */
    __iconBadge?: number[];
  }
}

// ── Helpers ─────────────────────────────────────────────────────────

/** "WIDTHxHEIGHT" of a PNG file, read from its header. */
function pngSize(file: Buffer): string {
  expect(file.subarray(1, 4).toString("latin1"), "a PNG file").toBe("PNG");
  return `${file.readUInt32BE(16)}x${file.readUInt32BE(20)}`;
}

/** Fetches an image the app names and checks it is a PNG of the stated size. */
async function expectPng(request: APIRequestContext, src: string, sizes: string): Promise<void> {
  const response = await request.get(src);
  expect(response.ok(), src).toBeTruthy();
  expect(response.headers()["content-type"], src).toContain("image/png");
  expect(pngSize(await response.body()), src).toBe(sizes);
}

/** Opens a page of the app and waits until the app itself is up (its code listens for the browser's install offer). */
async function openApp(page: Page, path: string): Promise<void> {
  await page.goto(path);
  await expect(mainNav(page)).toBeVisible();
}

/** Resolves once the service worker runs this page. */
async function controlled(page: Page): Promise<void> {
  await expect
    .poll(() => page.evaluate(() => navigator.serviceWorker.controller?.state ?? "none"))
    .toBe("activated");
}

/** Paths of everything the service worker has stored. */
function storedPaths(page: Page): Promise<string[]> {
  return page.evaluate(async () => {
    const paths: string[] = [];
    for (const name of await caches.keys()) {
      for (const request of await (await caches.open(name)).keys())
        paths.push(new URL(request.url).pathname);
    }
    return paths.sort();
  });
}

/**
 * What a browser does once it considers the site installable: it hands the
 * page an event whose `prompt()` opens the install dialog. `answer` is what
 * the person will pick in that dialog.
 */
async function browserOffersInstall(
  page: Page,
  answer: "accepted" | "dismissed" = "accepted",
): Promise<void> {
  await page.evaluate((outcome) => {
    const event = Object.assign(new Event("beforeinstallprompt", { cancelable: true }), {
      prompt: () => {
        window.__installPrompts = (window.__installPrompts ?? 0) + 1;
        return Promise.resolve();
      },
      userChoice: Promise.resolve({ outcome, platform: "web" }),
    });
    window.dispatchEvent(event);
  }, answer);
}

const installBanner = (page: Page) => page.getByRole("complementary", { name: "ホーム画面に追加" });
const mainNav = (page: Page) => page.getByRole("navigation", { name: "main" });
const toast = (page: Page, message: string) =>
  page.locator("[data-sonner-toast]").filter({ hasText: message });

/** Leave margin for a busy phone beyond the banner's 2.5-second startup timer. */
const SETTLED_MS = 5000;

// ── Manifest, icons, head ───────────────────────────────────────────

interface ManifestImage {
  src: string;
  sizes: string;
  type: string;
  purpose?: string;
  form_factor?: string;
  label?: string;
}

interface Manifest {
  id: string;
  name: string;
  short_name: string;
  description: string;
  lang: string;
  start_url: string;
  scope: string;
  display: string;
  display_override: string[];
  orientation: string;
  theme_color: string;
  background_color: string;
  categories: string[];
  icons: ManifestImage[];
  screenshots: ManifestImage[];
  shortcuts: { name: string; url: string; icons: ManifestImage[] }[];
}

base(
  "the manifest is complete, and every icon and screenshot it names is there in the stated size",
  async ({ request }) => {
    const response = await request.get("/manifest.webmanifest");
    expect(response.ok()).toBeTruthy();
    expect(response.headers()["content-type"]).toMatch(/manifest\+json|application\/json/);
    const manifest = (await response.json()) as Manifest;

    expect(manifest).toMatchObject({
      id: "/",
      name: "Tokuly",
      short_name: "Tokuly",
      lang: "ja",
      scope: "/",
      display: "standalone",
      orientation: "natural",
    });
    expect(manifest.description).toMatch(/位置情報共有/);
    expect(manifest.display_override).toContain("standalone");
    // Opens on the map, marked as coming from the installed app.
    expect(manifest.start_url).toBe("/map?source=pwa");
    // The page background of the light theme (src/styles.css).
    expect(manifest.theme_color).toBe("#f4f1ff");
    expect(manifest.background_color).toBe("#f4f1ff");
    expect(manifest.categories).toContain("social");

    // Installable needs 192 and 512 px icons; launchers that cut icons to a shape need maskable ones.
    for (const purpose of ["any", "maskable"]) {
      for (const sizes of ["192x192", "512x512"]) {
        expect(
          manifest.icons.filter(
            (icon) => icon.purpose === purpose && icon.sizes === sizes && icon.type === "image/png",
          ),
          `${purpose} ${sizes}`,
        ).toHaveLength(1);
      }
    }
    expect(manifest.icons.some((icon) => icon.purpose === "monochrome")).toBeTruthy();
    for (const icon of manifest.icons) {
      if (icon.type === "image/png") await expectPng(request, icon.src, icon.sizes);
      else
        expect((await request.get(icon.src)).headers()["content-type"], icon.src).toContain(
          icon.type,
        );
    }

    // The richer install dialog shows screenshots: phone-shaped on phones, wide ones on computers.
    for (const formFactor of ["narrow", "wide"]) {
      const shots = manifest.screenshots.filter((shot) => shot.form_factor === formFactor);
      expect(shots.length, formFactor).toBeGreaterThanOrEqual(1);
      // One form factor, one size: the dialog lays them out side by side.
      expect(new Set(shots.map((shot) => shot.sizes)).size, formFactor).toBe(1);
    }
    for (const shot of manifest.screenshots) {
      expect(shot.label, shot.src).toBeTruthy();
      await expectPng(request, shot.src, shot.sizes);
    }

    expect(
      manifest.shortcuts.map((shortcut) => new URL(shortcut.url, "http://tokuly.test").pathname),
    ).toEqual(["/map", "/chat", "/notes", "/notifications"]);
    for (const shortcut of manifest.shortcuts) {
      expect(shortcut.name).toBeTruthy();
      expect((await request.get(shortcut.url)).ok(), shortcut.url).toBeTruthy();
      for (const icon of shortcut.icons) await expectPng(request, icon.src, icon.sizes);
    }
  },
);

base(
  "every page links the manifest, the icons, and an iPhone launch image per screen and colour scheme",
  async ({ page, request }) => {
    await page.goto("/");
    await expect(page.locator('link[rel="manifest"]')).toHaveAttribute(
      "href",
      "/manifest.webmanifest",
    );
    for (const name of ["mobile-web-app-capable", "apple-mobile-web-app-capable"]) {
      await expect(page.locator(`meta[name="${name}"]`)).toHaveAttribute("content", "yes");
    }
    await expect(
      page.locator('meta[name="apple-mobile-web-app-status-bar-style"]'),
    ).toHaveAttribute("content", "default");
    // The bars around the page take the page background of the device's colour scheme.
    const themeColors = await page
      .locator('meta[name="theme-color" i]')
      .evaluateAll((tags) =>
        tags.map((tag) => [tag.getAttribute("media"), tag.getAttribute("content")]),
      );
    expect(themeColors).toEqual([
      ["(prefers-color-scheme: light)", "#f4f1ff"],
      ["(prefers-color-scheme: dark)", "#0d0b14"],
    ]);

    await expectPng(
      request,
      (await page.locator('link[rel="apple-touch-icon"]').getAttribute("href")) ?? "",
      "180x180",
    );
    for (const icon of await page.locator('link[rel="icon"][type="image/png"]').all()) {
      await expectPng(
        request,
        (await icon.getAttribute("href")) ?? "",
        (await icon.getAttribute("sizes")) ?? "",
      );
    }
    expect((await request.get("/favicon.ico")).ok()).toBeTruthy();

    const launchImages = await page
      .locator('link[rel="apple-touch-startup-image"]')
      .evaluateAll((links) =>
        links.map((link) => ({
          href: link.getAttribute("href") ?? "",
          media: link.getAttribute("media") ?? "",
        })),
      );
    expect(launchImages.length).toBeGreaterThanOrEqual(20);
    for (const scheme of ["light", "dark"]) {
      expect(
        launchImages.filter((image) => image.media.includes(`(prefers-color-scheme: ${scheme})`)),
      ).toHaveLength(launchImages.length / 2);
    }
    for (const { href, media } of launchImages) {
      // The image fills the screen the link is for: its points times the pixel ratio.
      const [, width, height, ratio] =
        /device-width: (\d+)px\) and \(device-height: (\d+)px\) and \(-webkit-device-pixel-ratio: (\d)\)/.exec(
          media,
        ) ?? [];
      await expectPng(
        request,
        href,
        `${Number(width) * Number(ratio)}x${Number(height) * Number(ratio)}`,
      );
    }
  },
);

test("the bars around the page follow the theme the app is set to, not only the device's", async ({
  page,
  me: _me,
}) => {
  await page.emulateMedia({ colorScheme: "dark" });
  await openApp(page, "/settings/appearance");
  const themeColors = () =>
    page
      .locator('meta[name="theme-color" i]')
      .evaluateAll((tags) => tags.map((tag) => tag.getAttribute("content")));
  await expect.poll(themeColors).toEqual(["#0d0b14", "#0d0b14"]);

  // A light app on a dark device: the bars go with the app.
  await page.getByRole("radio", { name: "ライト" }).click();
  await expect.poll(themeColors).toEqual(["#f4f1ff", "#f4f1ff"]);
});

// ── Service worker ──────────────────────────────────────────────────

test("the service worker takes over the app and keeps only public, static files", async ({
  page,
  me: _me,
}) => {
  await page.goto("/me");
  await expect(page.getByRole("heading", { name: "じぶん" })).toBeVisible();
  await controlled(page);
  await expect.poll(() => storedPaths(page)).toContain("/offline.html");

  // A second load goes through the worker; then look at what it kept.
  await page.reload();
  await expect(page.getByRole("heading", { name: "じぶん" })).toBeVisible();
  await page.waitForLoadState("networkidle");
  const stored = await storedPaths(page);
  for (const path of stored)
    expect(path, "only the offline page, built assets and map assets are stored").toMatch(
      /^\/(offline\.html$|assets\/|map-assets\/)/,
    );

  // A production build serves its scripts from /assets/ (the development server has none): those are kept.
  const builtAssets = await page.evaluate(
    () =>
      performance
        .getEntriesByType("resource")
        .filter((entry) => new URL(entry.name).pathname.startsWith("/assets/")).length,
  );
  if (builtAssets > 0)
    expect(stored.filter((path) => path.startsWith("/assets/")).length).toBeGreaterThan(0);
});

test("without a connection a page load ends on the offline page, which brings the app back by itself", async ({
  page,
  context,
  me: _me,
}) => {
  await page.goto("/me");
  await expect(page.getByRole("heading", { name: "じぶん" })).toBeVisible();
  await controlled(page);
  await expect.poll(() => storedPaths(page)).toContain("/offline.html");

  await context.setOffline(true);
  await page.goto("/friends");
  await expect(page.getByRole("heading", { name: "オフラインみたい…" })).toBeVisible();
  // It stands in for the page that was asked for.
  await expect(page).toHaveURL(/\/friends$/);

  // Trying again without a connection says so and stays.
  await page.getByRole("button", { name: "もう一度ためす" }).click();
  await expect(page.getByText("まだつながらないみたい")).toBeVisible();

  await context.setOffline(false);
  await expect(page.getByRole("heading", { level: 1, name: "ともだち" })).toBeVisible();
});

test("the offline page speaks English to people who use the app in English", async ({
  page,
  context,
  baseURL,
  me: _me,
}) => {
  await context.addCookies([{ name: "tokuly_lang", value: "en", url: baseURL ?? "" }]);
  await page.goto("/me");
  await controlled(page);
  await expect.poll(() => storedPaths(page)).toContain("/offline.html");

  await context.setOffline(true);
  await page.goto("/friends");
  await expect(page.getByRole("heading", { name: "You're offline" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Try again" })).toBeVisible();
  await context.setOffline(false);
  await expect(page.getByRole("heading", { level: 1, name: "Friends" })).toBeVisible();
});

test("a pill says so while the device is offline, and only then", async ({
  page,
  context,
  me: _me,
}) => {
  await page.goto("/me");
  await expect(page.getByRole("heading", { name: "じぶん" })).toBeVisible();
  const pill = page.getByRole("status").filter({ hasText: "オフラインだよ" });
  await expect(pill).toHaveCount(0);

  await context.setOffline(true);
  await expect(pill).toBeVisible();
  await context.setOffline(false);
  await expect(pill).toHaveCount(0);
});

test("a new version announces itself and takes over only when asked", async ({ page, me: _me }) => {
  await openApp(page, "/me");
  await controlled(page);
  const running = () => page.evaluate(() => navigator.serviceWorker.controller?.scriptURL ?? "");
  expect(await running()).toMatch(/\/sw\.js$/);

  // A different worker script for the app: to the browser, a new version.
  await page.evaluate(() => navigator.serviceWorker.register("/sw.js?version=next"));

  // It waits: the page keeps the worker it has, and says there is something new.
  const offer = toast(page, "新しいバージョンがあるよ！");
  await expect(offer).toBeVisible();
  expect(
    await page.evaluate(() =>
      navigator.serviceWorker
        .getRegistration()
        .then((registration) => registration?.waiting?.state),
    ),
  ).toBe("installed");
  expect(await running()).toMatch(/\/sw\.js$/);

  // "Update" hands over and reloads the page into the new version.
  const reloaded = page.waitForEvent("load");
  await offer.getByRole("button", { name: "更新する" }).click();
  await reloaded;
  await expect(mainNav(page)).toBeVisible();
  expect(await running()).toMatch(/\/sw\.js\?version=next$/);
});

test("an update accepted in another tab is offered here too, as a reload", async ({
  page,
  context,
  me: _me,
}) => {
  await openApp(page, "/me");
  await controlled(page);
  const other = await context.newPage();
  await openApp(other, "/me");
  await controlled(other);

  await other.evaluate(() => navigator.serviceWorker.register("/sw.js?version=next"));
  await expect(toast(page, "新しいバージョンがあるよ！")).toBeVisible();
  const otherReloaded = other.waitForEvent("load");
  await toast(other, "新しいバージョンがあるよ！")
    .getByRole("button", { name: "更新する" })
    .click();
  await otherReloaded;

  // The new worker now runs this page as well, but the page itself is still the old one: the offer stays.
  await expect
    .poll(() => page.evaluate(() => navigator.serviceWorker.controller?.scriptURL ?? ""))
    .toMatch(/\/sw\.js\?version=next$/);
  await expect(toast(page, "新しいバージョンがあるよ！")).toBeVisible();
  const reloaded = page.waitForEvent("load");
  await toast(page, "新しいバージョンがあるよ！").getByRole("button", { name: "更新する" }).click();
  await reloaded;
  await expect(mainNav(page)).toBeVisible();
});

test("'later' leaves the running version alone, and the app page keeps offering the update", async ({
  page,
  me: _me,
}) => {
  await openApp(page, "/me");
  await controlled(page);
  await page.evaluate(() => navigator.serviceWorker.register("/sw.js?version=next"));
  await expect
    .poll(() =>
      page.evaluate(() =>
        navigator.serviceWorker
          .getRegistration()
          .then((registration) => registration?.waiting?.state),
      ),
    )
    .toBe("installed");
  const offer = toast(page, "新しいバージョンがあるよ！");
  await expect(offer).toBeVisible();
  await offer.getByRole("button", { name: "あとで" }).click();
  await expect(offer).toHaveCount(0);

  await page.getByRole("main").getByRole("link", { name: "設定" }).click();
  await page
    .getByRole("navigation", { name: "設定" })
    .getByRole("link", { name: /^アプリとして使う/ })
    .click();
  await expect(page.getByText("更新あり")).toBeVisible();
  expect(await page.evaluate(() => navigator.serviceWorker.controller?.scriptURL ?? "")).toMatch(
    /\/sw\.js$/,
  );
  await expect(page.getByRole("main").getByRole("button", { name: "更新する" })).toBeVisible();
});

// ── Installing ──────────────────────────────────────────────────────

test("the offer to install waits for a calm page, and one tap opens the browser's dialog", async ({
  page,
  me: _me,
}) => {
  await openApp(page, "/map");
  await browserOffersInstall(page);

  // Never over the map.
  await page.waitForTimeout(SETTLED_MS);
  await expect(installBanner(page)).toHaveCount(0);

  await mainNav(page).getByRole("link", { name: "じぶん" }).click();
  await expect(page.getByRole("heading", { name: "じぶん" })).toBeVisible();
  // It floats above the page: nothing moves when it comes.
  const placed = () =>
    page.evaluate(() =>
      [...document.querySelectorAll("main a, main h1")].map((element) =>
        JSON.stringify(element.getBoundingClientRect()),
      ),
    );
  const before = await placed();
  await expect(installBanner(page)).toBeVisible();
  expect(await placed()).toEqual(before);
  await installBanner(page).getByRole("button", { name: "追加する" }).click();
  await expect.poll(() => page.evaluate(() => window.__installPrompts)).toBe(1);
  // Installed: nothing left to offer.
  await expect(installBanner(page)).toHaveCount(0);
});

test("'later' keeps the offer away, also after a reload, while the settings go on offering", async ({
  page,
  me: _me,
}) => {
  await openApp(page, "/me");
  await browserOffersInstall(page);
  await installBanner(page).getByRole("button", { name: "あとで" }).click();
  await expect(installBanner(page)).toHaveCount(0);

  await page.reload();
  await expect(mainNav(page)).toBeVisible();
  await browserOffersInstall(page);
  await page.waitForTimeout(SETTLED_MS);
  await expect(installBanner(page)).toHaveCount(0);

  // Through the settings to the app page, without loading a new document (that would drop the browser's offer).
  await page.getByRole("main").getByRole("link", { name: "設定" }).click();
  await expect(installBanner(page)).toHaveCount(0);
  await page
    .getByRole("navigation", { name: "設定" })
    .getByRole("link", { name: /^アプリとして使う/ })
    .click();
  await expect(page).toHaveURL(/\/pwa$/);
  await page.getByRole("button", { name: "インストールする" }).click();
  await expect.poll(() => page.evaluate(() => window.__installPrompts)).toBe(1);
});

test("answering 'no' in the browser's dialog counts as 'later'", async ({ page, me: _me }) => {
  await openApp(page, "/me");
  await browserOffersInstall(page, "dismissed");
  await installBanner(page).getByRole("button", { name: "追加する" }).click();
  await expect(installBanner(page)).toHaveCount(0);

  await page.reload();
  await expect(mainNav(page)).toBeVisible();
  await browserOffersInstall(page);
  await page.waitForTimeout(SETTLED_MS);
  await expect(installBanner(page)).toHaveCount(0);
});

test("opened as the installed app, it says so and offers nothing", async ({ page, me: _me }) => {
  // How an app opened from the iPhone home screen tells the page (`display-mode` cannot be set from here).
  await page.addInitScript(() => Object.defineProperty(navigator, "standalone", { value: true }));

  await openApp(page, "/me");
  await browserOffersInstall(page);
  await page.waitForTimeout(SETTLED_MS);
  await expect(installBanner(page)).toHaveCount(0);

  await openApp(page, "/pwa");
  await expect(page.getByRole("heading", { name: "アプリとして使ってるよ！" })).toBeVisible();
  await expect(page.getByText("済み", { exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "インストールする" })).toHaveCount(0);
});

test.describe("on an iPhone, where the browser has no install dialog", () => {
  const IPHONE =
    "Mozilla/5.0 (iPhone; CPU iPhone OS 18_5 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko)";

  test.describe("in Safari", () => {
    test.use({ userAgent: `${IPHONE} Version/18.5 Mobile/15E148 Safari/604.1` });

    test("the offer shows the way through the share menu, step by step", async ({
      page,
      me: _me,
    }) => {
      await openApp(page, "/me");
      await installBanner(page).getByRole("button", { name: "やり方" }).click();
      const guide = page.getByRole("dialog", { name: "ホーム画面に追加するには" });
      for (const step of [
        "共有ボタンをタップ",
        "「ホーム画面に追加」をえらぶ",
        "右上の「追加」をタップ",
      ])
        await expect(guide.getByText(step)).toBeVisible();
      await expect(guide.getByText("まずはSafariで開こう")).toHaveCount(0);
      await guide.getByRole("button", { name: "わかった！" }).click();
      await expect(guide).toHaveCount(0);

      // The app page opens the same guide.
      await openApp(page, "/pwa");
      await page.getByRole("button", { name: "追加のしかたを見る" }).click();
      await expect(guide).toBeVisible();
    });
  });

  test.describe("inside another app", () => {
    test.use({
      userAgent: `${IPHONE} Mobile/15E148 Safari Line/15.9.0`,
      permissions: ["clipboard-read", "clipboard-write"],
    });

    test("the offer first sends the page to Safari, with a link to take along", async ({
      page,
      baseURL,
      me: _me,
    }) => {
      await openApp(page, "/me");
      await expect(installBanner(page).getByText("Safariで開くと追加できるよ")).toBeVisible();
      await installBanner(page).getByRole("button", { name: "やり方" }).click();
      const guide = page.getByRole("dialog", { name: "ホーム画面に追加するには" });
      await expect(guide.getByText("まずはSafariで開こう")).toBeVisible();
      await guide.getByRole("button", { name: "リンクをコピー" }).click();
      await expect(toast(page, "コピーしたよ！")).toBeVisible();
      expect(await page.evaluate(() => navigator.clipboard.readText())).toBe(`${baseURL}/`);
    });
  });
});

// ── App badge ───────────────────────────────────────────────────────

test("the app's icon carries the unread count, and loses it when everything is read", async ({
  page,
  request,
  me,
}) => {
  // Stand-ins for the Badging API, which only installed apps have.
  await page.addInitScript(() => {
    const shown: number[] = [];
    window.__iconBadge = shown;
    Object.assign(navigator, {
      setAppBadge: (count?: number) => {
        shown.push(count ?? -1);
        return Promise.resolve();
      },
      clearAppBadge: () => {
        shown.push(0);
        return Promise.resolve();
      },
    });
  });
  const haru = await createAccount(request, "haru");
  await rpc(
    request,
    "tokuly.social.v1.FriendService/SendFriendRequest",
    { userId: me.userId },
    haru,
  );

  await page.goto("/me");
  const onIcon = () => page.evaluate(() => window.__iconBadge?.at(-1));
  await expect.poll(onIcon).toBe(1);

  await rpc(request, "tokuly.notify.v1.InboxService/MarkRead", { all: true }, me);
  await expect.poll(onIcon).toBe(0);
});

// ── The app page ────────────────────────────────────────────────────

test("the settings lead to the app page, which says how the app stands on this device", async ({
  page,
  me: _me,
}) => {
  await page.goto("/settings");
  await page
    .getByRole("navigation", { name: "設定" })
    .getByRole("link", { name: /^アプリとして使う/ })
    .click();
  await expect(page).toHaveURL(/\/pwa$/);
  await expect(page.getByRole("heading", { level: 1, name: "アプリとして使う" })).toBeVisible();

  // In a browser tab, with nothing to prompt with: how to install by hand.
  await expect(page.getByText("いまはブラウザで開いてるよ。")).toBeVisible();
  await expect(page.getByText("ブラウザのメニューから「アプリをインストール」")).toBeVisible();
  // The worker runs and has the offline page.
  await expect(page.getByText("準備OK")).toBeVisible();
  await expect(page.getByText("最新", { exact: true })).toBeVisible();
  await expect(page.getByRole("link", { name: "通知設定をひらく" })).toHaveAttribute(
    "href",
    "/notifications/settings",
  );
  await expect(page.getByRole("progressbar", { name: "ストレージの使用量" })).toBeVisible();

  await page.getByRole("button", { name: "確認する" }).click();
  await expect(toast(page, "最新バージョンだよ！")).toBeVisible();

  await expect(page.getByRole("link", { name: "back" })).toHaveAttribute("href", "/settings");
});

test("offline data can be cleared, and the offline page is ready again right away", async ({
  page,
  me: _me,
}) => {
  await page.goto("/pwa");
  await controlled(page);
  // Something the worker would have stored along the way.
  await page.evaluate(async () =>
    (await caches.open("tokuly-map-v1")).put("/map-assets/e2e.json", new Response("{}")),
  );
  await page.reload();
  await expect
    .poll(() => storedPaths(page))
    .toEqual(expect.arrayContaining(["/map-assets/e2e.json", "/offline.html"]));

  await page.getByRole("button", { name: "オフラインデータを消す" }).click();
  await page
    .getByRole("dialog", { name: "オフラインデータを消す？" })
    .getByRole("button", { name: "消す" })
    .click();
  await expect(toast(page, "オフラインデータを消したよ！")).toBeVisible();
  // Gone, except the offline page; what the page needs from here on is stored anew (a font, in a production build).
  await expect.poll(() => storedPaths(page)).not.toContain("/map-assets/e2e.json");
  await expect.poll(() => storedPaths(page)).toContain("/offline.html");
  await expect(page.getByTestId("offline-files")).toHaveText(
    /すばやく開くために保存したファイル: \d+個/,
  );
});
