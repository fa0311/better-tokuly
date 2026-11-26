import { createServer } from "node:http";
import type { AddressInfo } from "node:net";
import type { APIRequestContext, Browser, Page } from "@playwright/test";
import {
  type Account,
  adminAccount,
  createAccount,
  expect,
  RpcError,
  rpc,
  test as signedIn,
  signInAs,
} from "./support";

const NEWS = "tokuly.feed.v1.NewsService";
const NEWS_ADMIN = "tokuly.feed.v1.NewsAdminService";

/** A short random word that makes what a test creates findable among everyone else's. */
const unique = (prefix: string) =>
  `${prefix}${Date.now().toString(36).slice(-5)}${Math.random().toString(36).slice(2, 6)}`;

/** An RSS feed of three invented articles, served from the test process. */
interface Feed {
  /** Address of the feed document. */
  url: string;
  /** Address of a second, empty feed on the same server (to find out whether the API can fetch from here). */
  probeUrl: string;
  /** The feed's own title: the source takes it as its name on the first fetch. */
  title: string;
  /** A category every article carries (and nothing else in the database does). */
  tag: string;
  /** Titles of the articles, newest first. */
  titles: [string, string, string];
  /** Addresses of the articles on the "site", newest first. */
  links: [string, string, string];
}

const escapeXml = (text: string) =>
  text.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");

/**
 * `test` with a feed server on 127.0.0.1 for the duration of a test. The API
 * only fetches loopback addresses in development (`TOKULY_NEWS_ALLOW_LOOPBACK`):
 * tests that need articles call `requireLoopbackFeeds` and are skipped elsewhere.
 */
const test = signedIn.extend<{ feed: Feed }>({
  feed: async ({ request: _request }, use) => {
    const id = unique("");
    const tag = `e2e${id}`;
    const title = `E2Eタイムズ ${id}`;
    const titles: Feed["titles"] = [
      `${id} 文化祭の準備がはじまった`,
      `${id} 放課後カフェの新メニュー`,
      `${id} テスト勉強のコツ`,
    ];
    const server = createServer((incoming, response) => {
      const origin = `http://127.0.0.1:${(server.address() as AddressInfo).port}`;
      if (incoming.url?.startsWith("/probe")) {
        response.writeHead(200, { "content-type": "application/rss+xml; charset=utf-8" });
        response.end(
          `<?xml version="1.0" encoding="UTF-8"?><rss version="2.0"><channel><title>probe</title><link>${origin}/</link><description>probe</description></channel></rss>`,
        );
        return;
      }
      if (incoming.url?.startsWith("/feed")) {
        const items = titles
          .map((item, index) => {
            const published = new Date(Date.now() - (index + 1) * 60_000).toUTCString();
            return `<item><title>${escapeXml(item)}</title><link>${origin}/articles/${index + 1}</link><guid>${origin}/articles/${index + 1}</guid><pubDate>${published}</pubDate><description>${escapeXml(`${item}。くわしくは本文で！`)}</description><category>${tag}</category></item>`;
          })
          .join("");
        response.writeHead(200, { "content-type": "application/rss+xml; charset=utf-8" });
        response.end(
          `<?xml version="1.0" encoding="UTF-8"?><rss version="2.0"><channel><title>${escapeXml(title)}</title><link>${origin}/</link><description>e2e</description><language>ja</language>${items}</channel></rss>`,
        );
        return;
      }
      response.writeHead(200, { "content-type": "text/html; charset=utf-8" });
      response.end(
        `<!doctype html><title>${escapeXml(title)}</title><h1>${escapeXml(incoming.url ?? "")}</h1>`,
      );
    });
    await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
    const origin = `http://127.0.0.1:${(server.address() as AddressInfo).port}`;
    await use({
      url: `${origin}/feed-${id}.xml`,
      probeUrl: `${origin}/probe-${id}.xml`,
      title,
      tag,
      titles,
      links: [`${origin}/articles/1`, `${origin}/articles/2`, `${origin}/articles/3`],
    });
    await new Promise((resolve) => server.close(resolve));
  },
});

interface Article {
  id: string;
  title: string;
  clicked?: boolean;
  feedback?: string;
}

interface ManagedSource {
  id: string;
  name: string;
  feedUrl: string;
  enabled?: boolean;
  /** The feed is listed in the server's `TOKULY_NEWS_FEEDS`. */
  configured?: boolean;
  lastFetchedAt?: string;
  lastError?: string;
}

const managedSources = async (request: APIRequestContext, admin: Account) =>
  (
    await rpc<{ sources?: ManagedSource[] }>(
      request,
      `${NEWS_ADMIN}/ListManagedNewsSources`,
      {},
      admin,
    )
  ).sources ?? [];

/**
 * Skips the test where the API cannot fetch a feed from this machine: it
 * refuses loopback addresses outside development, and in a container its own
 * loopback is not this one. An empty feed is fetched once to find out.
 */
async function requireLoopbackFeeds(
  request: APIRequestContext,
  admin: Account,
  feed: Feed,
): Promise<void> {
  // Feeds are fetched in the background, twice per test (the probe, then the real one): allow for it.
  test.slow();
  let probeId: string;
  try {
    probeId = (
      await rpc<{ source: { id: string } }>(
        request,
        `${NEWS_ADMIN}/CreateNewsSource`,
        { feedUrl: feed.probeUrl },
        admin,
      )
    ).source.id;
  } catch (error) {
    if (!(error instanceof RpcError) || error.reason !== "FEED_URL_NOT_ALLOWED") throw error;
    test.skip(
      true,
      "the API does not fetch loopback feeds; run it with TOKULY_NEWS_ALLOW_LOOPBACK=true",
    );
    return;
  }
  const probe = async () =>
    (await managedSources(request, admin)).find((source) => source.id === probeId);
  await expect
    .poll(async () => Boolean((await probe())?.lastFetchedAt), {
      timeout: 30_000,
      message: "the probe feed is fetched",
    })
    .toBe(true);
  const failure = (await probe())?.lastError;
  await rpc(request, `${NEWS_ADMIN}/DeleteNewsSource`, { sourceId: probeId }, admin);
  test.skip(Boolean(failure), `the API cannot reach a feed on this machine: ${failure}`);
}

/** The articles of one source, newest first, as `as` sees them. */
async function articlesOf(
  request: APIRequestContext,
  sourceId: string,
  as: Account,
): Promise<Article[]> {
  return (
    (
      await rpc<{ articles?: Article[] }>(
        request,
        `${NEWS}/ListNews`,
        { sourceId, page: { limit: 20 } },
        as,
      )
    ).articles ?? []
  );
}

/** Adds the feed as a source through the API and waits until its three articles arrived. */
async function addSource(
  request: APIRequestContext,
  admin: Account,
  feed: Feed,
): Promise<{ sourceId: string; articles: Article[] }> {
  const created = await rpc<{ source: { id: string } }>(
    request,
    `${NEWS_ADMIN}/CreateNewsSource`,
    { feedUrl: feed.url },
    admin,
  );
  const sourceId = created.source.id;
  await expect
    .poll(async () => (await articlesOf(request, sourceId, admin)).length, {
      timeout: 30_000,
      message: "the feed's articles arrive",
    })
    .toBe(3);
  return { sourceId, articles: await articlesOf(request, sourceId, admin) };
}

/** Removes a test's source (and with it its articles); it may be gone already. */
async function dropSource(
  request: APIRequestContext,
  admin: Account,
  sourceId: string | undefined,
): Promise<void> {
  if (sourceId)
    await rpc(request, `${NEWS_ADMIN}/DeleteNewsSource`, { sourceId }, admin).catch(() => {});
}

/** A second browser session, signed in as the deployment's admin, next to the test's own. */
async function adminSession(
  browser: Browser,
  admin: Account,
): Promise<{ staff: Page; close(): Promise<void> }> {
  const { baseURL, locale, timezoneId, viewport, hasTouch } = test.info().project.use;
  const context = await browser.newContext({ baseURL, locale, timezoneId, viewport, hasTouch });
  const staff = await context.newPage();
  await signInAs(staff, admin);
  return { staff, close: () => context.close() };
}

const cards = (page: Page) => page.getByTestId("news-article");
const sourceRows = (page: Page) => page.getByTestId("news-source");

test.describe("reading the news", () => {
  test("staff add a feed; its articles arrive; a reader opens one and says what they think", async ({
    page,
    request,
    me,
    browser,
    feed,
  }) => {
    const admin = await adminAccount(request);
    await requireLoopbackFeeds(request, admin, feed);
    const { staff, close } = await adminSession(browser, admin);
    let sourceId: string | undefined;
    try {
      // Staff: add the feed and watch its first fetch come in.
      await staff.goto("/admin/news");
      await staff.getByRole("button", { name: "ソースを追加" }).first().click();
      const dialog = staff.getByRole("dialog");
      await dialog.getByLabel("フィードURL").fill(feed.url);
      await dialog.getByLabel("デフォルトタグ").fill("E2E部");
      await dialog.getByRole("button", { name: "ソースを追加" }).first().click();
      const row = sourceRows(staff).filter({ hasText: feed.url });
      await expect(row).toContainText(feed.title, { timeout: 30_000 });
      await expect(row).toContainText("正常");
      await expect(row).toContainText("#e2e部");
      sourceId = (await managedSources(request, admin)).find(
        (source) => source.feedUrl === feed.url,
      )?.id;
      expect(sourceId).toBeTruthy();

      await row.getByRole("button", { name: `${feed.title} の操作` }).click();
      await staff.getByRole("menuitem", { name: "今すぐ取得" }).click();
      await expect(staff.getByText(`${feed.title} の取得を予約したよ`)).toBeVisible();

      // Reader: the newest article is in the list, with its source.
      await page.goto("/news?order=latest");
      const card = cards(page).filter({ hasText: feed.titles[0] });
      await expect(card).toContainText(feed.title);
      await card.getByRole("link", { name: feed.titles[0] }).click();
      await expect(page).toHaveURL(/\/news\/[0-9a-f-]{36}$/);
      const articleId = page.url().split("/").pop() as string;

      // The article page: what it is about, where it comes from, and the way out to the full text.
      await expect(page.getByRole("heading", { level: 2, name: feed.titles[0] })).toBeVisible();
      await expect(page.getByText("くわしくは本文で！")).toBeVisible();
      await expect(
        page.getByRole("list", { name: "タグ" }).getByRole("link", { name: `#${feed.tag}` }),
      ).toBeVisible();
      const readLink = page.getByRole("link", { name: /記事を読む/ });
      await expect(readLink).toHaveAttribute("href", feed.links[0]);
      const [original] = await Promise.all([page.waitForEvent("popup"), readLink.click()]);
      await expect(original).toHaveURL(feed.links[0]);
      await original.close();

      // Opening the page is what counts as reading it.
      const article = async () =>
        (await rpc<{ article: Article }>(request, `${NEWS}/GetNewsArticle`, { articleId }, me))
          .article;
      await expect
        .poll(async () => (await article()).clicked, { message: "the click is recorded" })
        .toBe(true);

      // Feedback shows at once, reaches the API, and can be taken back.
      const more = page.getByRole("button", { name: "もっと見たい" });
      await more.click();
      await expect(more).toHaveAttribute("aria-pressed", "true");
      // (The offline indicator is a status of its own, empty while there is a connection.)
      await expect(
        page.getByRole("status").filter({ hasText: "りょうかい！似たニュースを増やすね" }),
      ).toBeVisible();
      await expect.poll(async () => (await article()).feedback).toBe("NEWS_FEEDBACK_MORE");

      await page.goBack();
      const sameCard = cards(page).filter({ hasText: feed.titles[0] });
      await expect(sameCard.getByRole("button", { name: "もっと見たい" })).toHaveAttribute(
        "aria-pressed",
        "true",
      );
      await sameCard.getByRole("button", { name: "少なめに" }).click();
      await expect(sameCard.getByRole("button", { name: "少なめに" })).toHaveAttribute(
        "aria-pressed",
        "true",
      );
      await expect(sameCard.getByRole("button", { name: "もっと見たい" })).toHaveAttribute(
        "aria-pressed",
        "false",
      );
      await expect.poll(async () => (await article()).feedback).toBe("NEWS_FEEDBACK_LESS");
      await sameCard.getByRole("button", { name: "少なめに" }).click();
      await expect
        .poll(async () => (await article()).feedback ?? "NEWS_FEEDBACK_UNSPECIFIED")
        .toBe("NEWS_FEEDBACK_UNSPECIFIED");
    } finally {
      await close();
      await dropSource(request, admin, sourceId);
    }
  });

  test("tags and sources filter the list, and the filter lives in the address", async ({
    page,
    request,
    me: _me,
    feed,
  }) => {
    const admin = await adminAccount(request);
    await requireLoopbackFeeds(request, admin, feed);
    const { sourceId, articles } = await addSource(request, admin, feed);
    try {
      // From an article, its tag leads to everything carrying it.
      await page.goto(`/news/${articles[0]?.id}`);
      await page
        .getByRole("list", { name: "タグ" })
        .getByRole("link", { name: `#${feed.tag}` })
        .click();
      await expect(page).toHaveURL(
        (url) => url.pathname === "/news" && url.searchParams.get("tag") === feed.tag,
      );
      await expect(cards(page)).toHaveCount(3);
      const tags = page.getByRole("navigation", { name: "タグで絞り込む" });
      await expect(tags.getByRole("link", { name: `#${feed.tag}` })).toHaveAttribute(
        "aria-current",
        "page",
      );
      // A reload shows the same list: the address is all there is.
      await page.reload();
      await expect(cards(page)).toHaveCount(3);
      await tags.getByRole("link", { name: "すべて" }).click();
      await expect(page).toHaveURL((url) => !url.searchParams.has("tag"));

      // The sources page lists where articles come from; a source opens its own articles.
      await page.goto("/news/sources");
      await page
        .getByRole("listitem")
        .filter({ hasText: feed.title })
        .getByRole("link")
        .first()
        .click();
      await expect(page).toHaveURL((url) => url.searchParams.get("source") === sourceId);
      await expect(cards(page)).toHaveCount(3);
      for (const title of feed.titles)
        await expect(cards(page).filter({ hasText: title })).toBeVisible();
      await page.getByRole("link", { name: "提供元の絞り込みを外す" }).click();
      await expect(page).toHaveURL((url) => !url.searchParams.has("source"));
    } finally {
      await dropSource(request, admin, sourceId);
    }
  });

  test("what a reader opens shapes their interests, and they can start over", async ({
    page,
    request,
    me,
    feed,
  }) => {
    const admin = await adminAccount(request);
    await requireLoopbackFeeds(request, admin, feed);
    const { sourceId, articles } = await addSource(request, admin, feed);
    try {
      await page.goto(`/news/${articles[0]?.id}`);
      await page.getByRole("button", { name: "もっと見たい" }).click();
      await expect(page.getByRole("status").filter({ hasText: /./ })).toBeVisible();

      await page.goto("/news/interests");
      const liked = page.getByRole("region", { name: "よく読むタグ" });
      await expect(liked.getByRole("link", { name: new RegExp(`#${feed.tag}`) })).toBeVisible();

      await page.getByRole("button", { name: "リセットする" }).click();
      const dialog = page.getByRole("dialog");
      await dialog.getByRole("switch").click();
      await dialog.getByRole("button", { name: "リセットする" }).click();
      await expect(page.getByText("おすすめをリセットしたよ！")).toBeVisible();
      await expect(liked).toContainText("まだデータがないよ");
      const after = await rpc<{ article: Article }>(
        request,
        `${NEWS}/GetNewsArticle`,
        { articleId: articles[0]?.id },
        me,
      );
      expect(after.article.clicked ?? false).toBe(false);
      expect(after.article.feedback ?? "NEWS_FEEDBACK_UNSPECIFIED").toBe(
        "NEWS_FEEDBACK_UNSPECIFIED",
      );
    } finally {
      await dropSource(request, admin, sourceId);
    }
  });

  test("a filter without articles says so and leads back; the header leads to the update log", async ({
    page,
    me: _me,
  }) => {
    await page.goto(`/news?tag=${unique("none")}`);
    await expect(page.getByText("この条件のニュースは見つからなかった…")).toBeVisible();
    await page.getByRole("link", { name: "絞り込みを外す" }).click();
    await expect(page).toHaveURL(/\/news$/);
    await page.getByRole("link", { name: "アップデート情報" }).first().click();
    await expect(page).toHaveURL(/\/announcements$/);
  });

  test("an article that is gone says so, and the server's own article address leads to the page", async ({
    page,
    me: _me,
  }) => {
    const missing = "01a200cd-8ef4-753d-aefe-000000000000";
    await page.goto(`/news/articles/${missing}`);
    await expect(page).toHaveURL(new RegExp(`/news/${missing}$`));
    await expect(page.getByText("このニュースはもう読めないみたい")).toBeVisible();
    await page.getByRole("link", { name: "ニュース一覧へ" }).click();
    await expect(page).toHaveURL(/\/news$/);
  });

  test("the hub has a news tile", async ({ page, me: _me }) => {
    await page.goto("/me");
    await page.getByRole("main").getByRole("link", { name: "ニュース" }).click();
    await expect(page).toHaveURL(/\/news$/);
    await expect(page.getByRole("heading", { level: 1, name: "ニュース" })).toBeVisible();
  });
});

test.describe("staff", () => {
  test("a source can be renamed, switched off and deleted, and single articles taken down", async ({
    page,
    request,
    feed,
  }) => {
    const admin = await adminAccount(request);
    await requireLoopbackFeeds(request, admin, feed);
    const reader = await createAccount(request, "reader");
    const { sourceId, articles } = await addSource(request, admin, feed);
    try {
      await signInAs(page, admin);
      await page.goto("/admin/news");
      await expect(
        page
          .getByRole("navigation", { name: "管理コンソール" })
          .getByRole("link", { name: "ニュース管理" }),
      ).toHaveAttribute("aria-current", "page");
      const row = sourceRows(page).filter({ hasText: feed.url });
      await expect(row).toContainText(feed.title);
      await expect(row.getByText("記事数")).toBeVisible();

      // Rename.
      const renamed = `${feed.title} 改`;
      await row.getByRole("button", { name: `${feed.title} の操作` }).click();
      await page.getByRole("menuitem", { name: "編集する" }).click();
      await page.getByRole("dialog").getByLabel("名前").fill(renamed);
      await page.getByRole("dialog").getByRole("button", { name: "保存する" }).click();
      await expect(row).toContainText(renamed);

      // Take one article down from the staff page: paste its address, check, confirm.
      const [first, second] = articles as [Article, Article];
      await page.getByLabel("記事のURLまたはID").fill(`https://tokuly.example/news/${first.id}`);
      await page.getByRole("button", { name: "記事を確認する" }).click();
      await expect(page.getByTestId("article-to-remove")).toContainText(first.title);
      await page
        .getByTestId("article-to-remove")
        .getByRole("button", { name: "記事を取り下げる" })
        .click();
      await page.getByRole("dialog").getByRole("button", { name: "記事を取り下げる" }).click();
      await expect(page.getByText("記事を取り下げたよ")).toBeVisible();
      await expect(
        rpc(request, `${NEWS}/GetNewsArticle`, { articleId: first.id }, reader),
      ).rejects.toMatchObject({ reason: "NEWS_ARTICLE_NOT_FOUND" });

      // ...and another one from its own page.
      await page.goto(`/news/${second.id}`);
      await page.getByRole("button", { name: "スタッフメニュー" }).click();
      await page.getByRole("menuitem", { name: "記事を取り下げる" }).click();
      await page.getByRole("dialog").getByRole("button", { name: "記事を取り下げる" }).click();
      await expect(page).toHaveURL(/\/news$/);
      expect((await articlesOf(request, sourceId, reader)).map((article) => article.title)).toEqual(
        [feed.titles[2]],
      );

      // Switched off, a source's articles leave the lists; switched on, they return.
      await page.goto("/admin/news");
      await row.getByRole("button", { name: `${renamed} の操作` }).click();
      await page.getByRole("menuitem", { name: "無効にする" }).click();
      await expect(row).toContainText("無効");
      expect(await articlesOf(request, sourceId, reader)).toHaveLength(0);
      await row.getByRole("button", { name: `${renamed} の操作` }).click();
      await page.getByRole("menuitem", { name: "有効にする" }).click();
      await expect(row).not.toContainText("無効");
      expect(await articlesOf(request, sourceId, reader)).toHaveLength(1);

      // Delete.
      await row.getByRole("button", { name: `${renamed} の操作` }).click();
      await page.getByRole("menuitem", { name: "削除する" }).click();
      await page.getByRole("dialog").getByRole("button", { name: "削除する" }).click();
      await expect(row).toHaveCount(0);
      expect((await managedSources(request, admin)).map((source) => source.id)).not.toContain(
        sourceId,
      );
    } finally {
      await dropSource(request, admin, sourceId);
    }
  });

  test("a source listed in the server settings is switched off, not deleted, and the console says why", async ({
    page,
    request,
  }) => {
    const admin = await adminAccount(request);
    const source = (await managedSources(request, admin)).find((source) => source.configured);
    test.skip(!source, "no source comes from the settings; run the API with TOKULY_NEWS_FEEDS");
    if (!source) return;
    const setEnabled = (enabled: boolean) =>
      rpc(request, `${NEWS_ADMIN}/UpdateNewsSource`, { sourceId: source.id, enabled }, admin);
    await setEnabled(true);
    try {
      // The API refuses, with a reason of its own.
      const refusal = await rpc(
        request,
        `${NEWS_ADMIN}/DeleteNewsSource`,
        { sourceId: source.id },
        admin,
      ).catch((error: unknown) => error);
      expect(refusal).toBeInstanceOf(RpcError);
      expect((refusal as RpcError).reason).toBe("NEWS_SOURCE_CONFIGURED");

      await signInAs(page, admin);
      await page.goto("/admin/news");
      const row = sourceRows(page).filter({ hasText: source.feedUrl });
      await expect(row).toContainText("設定にあり");
      await row.getByRole("button", { name: `${source.name} の操作` }).click();
      await page.getByRole("menuitem", { name: "削除する" }).click();
      const dialog = page.getByRole("dialog");
      await expect(dialog).toContainText("削除できないよ");
      await expect(dialog).toContainText("TOKULY_NEWS_FEEDS");
      await expect(dialog.getByRole("button", { name: "削除する" })).toHaveCount(0);

      // What it offers instead works, and the source stays.
      await dialog.getByRole("button", { name: "無効にする" }).click();
      await expect(dialog).toHaveCount(0);
      await expect(row).toContainText("無効");
      const after = (await managedSources(request, admin)).find((it) => it.id === source.id);
      expect(after?.enabled ?? false).toBe(false);
    } finally {
      await setEnabled(source.enabled ?? false);
    }
  });

  test("a feed the server will not fetch is refused with a reason, in the form", async ({
    page,
    request,
  }) => {
    await signInAs(page, await adminAccount(request));
    await page.goto("/admin/news");
    await page.getByRole("button", { name: "ソースを追加" }).first().click();
    const dialog = page.getByRole("dialog");
    await dialog.getByLabel("フィードURL").fill("ftp://example.com/feed.xml");
    await dialog.getByRole("button", { name: "ソースを追加" }).first().click();
    await expect(dialog.getByRole("alert")).toBeVisible();
    await expect(dialog.getByLabel("フィードURL")).toHaveAttribute("aria-invalid", "true");
  });

  test("accounts without a staff role are kept out, and the API refuses them too", async ({
    page,
    request,
    me,
  }) => {
    await page.goto("/admin/news");
    await expect(page.getByText("ここはスタッフ専用だよ")).toBeVisible();
    await expect(
      rpc(request, `${NEWS_ADMIN}/ListManagedNewsSources`, {}, me),
    ).rejects.toMatchObject({ reason: "FORBIDDEN" });
  });
});
