import type { APIRequestContext } from "@playwright/test";
import { type Account, adminAccount, expect, rpc, test } from "./support";

const ANNOUNCEMENTS = "tokuly.admin.v1.AnnouncementService";

/** A short random tag that makes a test's announcements findable among everyone else's. */
const tag = () => `${Date.now().toString(36).slice(-5)}${Math.random().toString(36).slice(2, 6)}`;

interface Draft {
  title: { ja: string; en?: string };
  body: { ja: string; en?: string };
  summary?: { ja: string; en?: string };
  category?:
    | "ANNOUNCEMENT_CATEGORY_UPDATE"
    | "ANNOUNCEMENT_CATEGORY_MAINTENANCE"
    | "ANNOUNCEMENT_CATEGORY_EVENT"
    | "ANNOUNCEMENT_CATEGORY_IMPORTANT";
  pinned?: boolean;
}

/**
 * The update log as staff fill it: announcements a test creates through the
 * API (published at once unless told otherwise, never notifying anyone), all
 * deleted again when the test is over.
 */
class UpdateLog {
  private readonly created: string[] = [];

  constructor(
    private readonly request: APIRequestContext,
    private readonly admin: Account,
  ) {}

  async add(draft: Draft, publish = true): Promise<string> {
    const { announcement } = await rpc<{ announcement: { id: string } }>(
      this.request,
      `${ANNOUNCEMENTS}/CreateAnnouncement`,
      draft,
      this.admin,
    );
    this.created.push(announcement.id);
    if (publish)
      await rpc(
        this.request,
        `${ANNOUNCEMENTS}/PublishAnnouncement`,
        { id: announcement.id },
        this.admin,
      );
    return announcement.id;
  }

  async clear(): Promise<void> {
    for (const id of this.created)
      await rpc(this.request, `${ANNOUNCEMENTS}/DeleteAnnouncement`, { id }, this.admin).catch(
        () => {},
      );
  }
}

const logTest = test.extend<{ updateLog: UpdateLog }>({
  updateLog: async ({ request }, use) => {
    const log = new UpdateLog(request, await adminAccount(request));
    await use(log);
    await log.clear();
  },
});

logTest(
  "the update log lists what is published, pinned announcements first, and opens one in full",
  async ({ page, updateLog, me: _me }) => {
    const suffix = tag();
    const pinned = `e2e ピン留め ${suffix}`;
    const newer = `e2e 新着 ${suffix}`;
    await updateLog.add({
      title: { ja: pinned },
      summary: { ja: "いちばん上に出るお知らせです" },
      body: { ja: "固定表示のテスト" },
      pinned: true,
    });
    await updateLog.add({
      title: { ja: newer },
      body: {
        ja: "## 変更点\n\n- **速く**なりました\n- くわしくは[ヘルプ](https://tokuly.example/help)へ\n\n<script>window.hacked = true</script>\n\n[あやしいリンク](javascript:alert(1))",
      },
      category: "ANNOUNCEMENT_CATEGORY_EVENT",
    });

    await page.goto("/announcements");
    await expect(page.getByRole("heading", { level: 1, name: "アップデート情報" })).toBeVisible();
    const pinnedCard = page.getByRole("article").filter({ hasText: pinned });
    const newerCard = page.getByRole("article").filter({ hasText: newer });
    await expect(pinnedCard).toContainText("ピン留め");
    await expect(pinnedCard).toContainText("いちばん上に出るお知らせです");
    await expect(newerCard).toContainText("イベント");
    // An empty summary falls back to the beginning of the body.
    await expect(newerCard).toContainText("変更点");

    // Published later, yet listed after the pinned one.
    const titles = await page
      .getByRole("main")
      .getByRole("heading", { level: 2 })
      .allTextContents();
    expect(titles.indexOf(pinned)).toBeGreaterThanOrEqual(0);
    expect(titles.indexOf(pinned)).toBeLessThan(titles.indexOf(newer));

    await page.getByRole("link", { name: newer }).click();
    await expect(page).toHaveURL(/\/announcements\/[0-9a-f-]{36}$/);
    const article = page.getByRole("article");
    await expect(article.getByRole("heading", { level: 1, name: newer })).toBeVisible();

    // The Markdown body is rendered: a heading, emphasis, a list, a link that opens outside the app.
    await expect(article.getByRole("heading", { level: 3, name: "変更点" })).toBeVisible();
    await expect(article.locator("strong")).toHaveText("速く");
    await expect(article.getByRole("listitem")).toHaveCount(2);
    await expect(article.getByRole("link", { name: "ヘルプ" })).toHaveAttribute(
      "href",
      "https://tokuly.example/help",
    );
    await expect(article.getByRole("link", { name: "ヘルプ" })).toHaveAttribute("target", "_blank");

    // Raw HTML is shown as text, never run; a script URL never becomes a link.
    await expect(article.getByText("<script>window.hacked = true</script>")).toBeVisible();
    expect(await page.evaluate(() => (window as { hacked?: boolean }).hacked)).toBeUndefined();
    await expect(article.getByText("あやしいリンク")).toBeVisible();
    await expect(article.getByRole("link", { name: "あやしいリンク" })).toHaveCount(0);

    await article.getByRole("link", { name: "アップデート情報にもどる" }).click();
    await expect(page).toHaveURL(/\/announcements$/);
  },
);

logTest("categories narrow the list", async ({ page, updateLog, me: _me }) => {
  const suffix = tag();
  const event = `e2e イベント ${suffix}`;
  const maintenance = `e2e メンテ ${suffix}`;
  await updateLog.add({
    title: { ja: event },
    body: { ja: "スタンプラリー開催！" },
    category: "ANNOUNCEMENT_CATEGORY_EVENT",
  });
  await updateLog.add({
    title: { ja: maintenance },
    body: { ja: "深夜にメンテナンスします" },
    category: "ANNOUNCEMENT_CATEGORY_MAINTENANCE",
  });

  await page.goto("/announcements");
  await expect(page.getByRole("link", { name: event })).toBeVisible();
  await expect(page.getByRole("link", { name: maintenance })).toBeVisible();

  await page.getByRole("tab", { name: "メンテナンス" }).click();
  await expect(page).toHaveURL(/category=2/);
  await expect(page.getByRole("link", { name: maintenance })).toBeVisible();
  await expect(page.getByRole("link", { name: event })).toHaveCount(0);

  await page.getByRole("tab", { name: "すべて" }).click();
  await expect(page.getByRole("link", { name: event })).toBeVisible();
});

logTest(
  "drafts stay out of the update log, and a missing announcement says so",
  async ({ page, updateLog, me: _me }) => {
    const draft = `e2e 下書き ${tag()}`;
    const id = await updateLog.add({ title: { ja: draft }, body: { ja: "まだ見せない" } }, false);

    await page.goto("/announcements");
    await expect(page.getByRole("heading", { level: 1, name: "アップデート情報" })).toBeVisible();
    await expect(page.getByRole("tab", { name: "すべて" })).toBeVisible();
    await expect(page.getByText(draft)).toHaveCount(0);

    await page.goto(`/announcements/${id}`);
    await expect(page.getByText("このお知らせは見つかりませんでした")).toBeVisible();
    await page.getByRole("link", { name: "アップデート情報にもどる" }).click();
    await expect(page).toHaveURL(/\/announcements$/);
  },
);

logTest(
  "readers in English get the English text, or the Japanese one where there is none",
  async ({ page, context, baseURL, updateLog, me: _me }) => {
    const suffix = tag();
    const both = await updateLog.add({
      title: { ja: `e2e 両方 ${suffix}`, en: `e2e Both ${suffix}` },
      body: { ja: "日本語の本文", en: "The English body" },
    });
    await updateLog.add({
      title: { ja: `e2e 日本語だけ ${suffix}` },
      body: { ja: "日本語しかない本文" },
    });
    await context.addCookies([{ name: "tokuly_lang", value: "en", url: baseURL as string }]);

    await page.goto("/announcements");
    await expect(page.getByRole("heading", { level: 1, name: "What's new" })).toBeVisible();
    await expect(page.getByRole("link", { name: `e2e Both ${suffix}` })).toBeVisible();
    await expect(page.getByRole("link", { name: `e2e 日本語だけ ${suffix}` })).toBeVisible();

    await page.goto(`/announcements/${both}`);
    await expect(page.getByRole("article").getByText("The English body")).toBeVisible();
  },
);
