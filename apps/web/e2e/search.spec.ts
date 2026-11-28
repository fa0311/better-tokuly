import type { APIRequestContext, Browser, Page } from "@playwright/test";
import {
  type Account,
  adminAccount,
  backOnline,
  befriend,
  createAccount,
  expect,
  failToLoad,
  rpc,
  signInAs,
  test,
} from "./support";

const SEARCH = "tokuly.search.v1.SearchService";
const SEARCH_ADMIN = "tokuly.search.v1.SearchAdminService";
const NOTES = "tokuly.notes.v1.NoteService";

/** A short random word that makes what a test creates findable among everyone else's. */
const unique = (prefix: string) =>
  `${prefix}${Date.now().toString(36).slice(-5)}${Math.random().toString(36).slice(2, 6)}`;

interface Hit {
  kind: string;
  id: string;
  title: string;
}

/** Searches through the API, as `as` (signed in, so `skipHistory` decides what is remembered). */
async function apiSearch(
  request: APIRequestContext,
  query: string,
  as: Account,
  options: { kind?: string; skipHistory?: boolean } = {},
): Promise<Hit[]> {
  const response = await rpc<{ hits?: Hit[] }>(
    request,
    `${SEARCH}/Search`,
    { query, kind: options.kind, skipHistory: options.skipHistory ?? true },
    as,
  );
  return response.hits ?? [];
}

/** Waits until the index has caught up with something that was just created. */
async function waitUntilFound(
  request: APIRequestContext,
  query: string,
  as: Account,
  kind?: string,
): Promise<void> {
  await expect
    .poll(async () => (await apiSearch(request, query, as, { kind })).length, {
      timeout: 30_000,
      message: `"${query}" becomes searchable`,
    })
    .toBeGreaterThan(0);
}

/** Posts a public note as `author` (agreeing to the posting rules first) and returns its ID. */
async function postNote(
  request: APIRequestContext,
  author: Account,
  text: string,
): Promise<string> {
  const rules = await rpc<{ version: number }>(request, `${NOTES}/GetNoteRules`, {}, author);
  await rpc(request, `${NOTES}/AgreeToNoteRules`, { version: rules.version }, author);
  const created = await rpc<{ note: { id: string } }>(
    request,
    `${NOTES}/CreateNote`,
    { text, visibility: "VISIBILITY_PUBLIC" },
    author,
  );
  return created.note.id;
}

const searchBox = (page: Page) => page.getByRole("combobox", { name: "検索", exact: true });
const hits = (page: Page) => page.getByTestId("search-hit");

/** Types a query into the page's search box and runs it. */
async function searchFor(page: Page, query: string): Promise<void> {
  await searchBox(page).fill(query);
  await searchBox(page).press("Enter");
  await expect(page).toHaveURL((url) => url.searchParams.get("q") === query);
}

/** Walks the suggestion list with the arrow keys until `option` is the selected one. */
async function arrowTo(page: Page, option: ReturnType<Page["getByRole"]>): Promise<void> {
  await expect(option).toBeVisible();
  for (
    let presses = 0;
    presses < 12 && (await option.getAttribute("aria-selected")) !== "true";
    presses++
  )
    await page.keyboard.press("ArrowDown");
  await expect(option).toHaveAttribute("aria-selected", "true");
}

test.describe("searching", () => {
  test("a person is found by typing their name, with the match highlighted, and the result opens their profile", async ({
    page,
    request,
    me,
  }) => {
    const name = unique("Zorb");
    const person = await createAccount(request, name);
    await waitUntilFound(request, name, me);

    await page.goto("/search");
    await searchFor(page, name);

    const card = hits(page).filter({ hasText: `@${person.handle}` });
    await expect(card).toBeVisible();
    await expect(card.locator("mark")).toHaveText(name);
    // The tab row offers the kind that matched, with its count.
    await expect(page.getByRole("tab", { name: /ひと/ })).toContainText("1");

    await card.click();
    await expect(page).toHaveURL(new RegExp(`/u/${person.handle}$`));
  });

  test("notes and hashtags are found, each in its own tab, and the tab lives in the address", async ({
    page,
    request,
    me,
  }) => {
    const author = await createAccount(request, "writer");
    const word = unique("mogu");
    const hashtag = unique("tagu");
    const noteId = await postNote(request, author, `${word} を食べたよ！ #${hashtag}`);
    await waitUntilFound(request, word, me, "SEARCH_KIND_NOTE");
    await waitUntilFound(request, `#${hashtag}`, me, "SEARCH_KIND_HASHTAG");

    await page.goto("/search");
    await searchFor(page, word);
    // The "all" tab groups results in sections by kind.
    const noteCard = page
      .getByRole("region", { name: "ノート", exact: true })
      .getByTestId("search-hit");
    await expect(noteCard).toHaveAttribute("href", `/notes/${noteId}`);
    await expect(noteCard.locator("mark").first()).toHaveText(word);
    await expect(noteCard).toContainText(author.displayName);

    // The note's own tab is a link: it changes the address, and the address alone restores it.
    await page.getByRole("tab", { name: /ノート/ }).click();
    await expect(page).toHaveURL((url) => url.searchParams.get("kind") === "notes");
    await expect(page.getByRole("tab", { name: /ノート/ })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    await page.reload();
    await expect(hits(page)).toHaveCount(1);
    await page.goBack();
    await expect(page.getByRole("tab", { name: "すべて" })).toHaveAttribute(
      "aria-selected",
      "true",
    );

    // A leading '#' searches tags only; the chip switches that on and off.
    await searchFor(page, `#${hashtag}`);
    await expect(page.getByRole("button", { name: "タグだけ" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    const tagCard = hits(page)
      .filter({ hasText: `#${hashtag}` })
      .first();
    await expect(tagCard).toBeVisible();
    await expect(page.locator(`[data-testid="search-hit"][data-kind="HASHTAG"]`)).toHaveAttribute(
      "href",
      `/tag/${hashtag}`,
    );
  });

  test("results can be ordered, and the order is part of the address", async ({
    page,
    request,
    me,
  }) => {
    const author = await createAccount(request, "sorter");
    const word = unique("narabi");
    const first = await postNote(request, author, `${word} いちばんめ`);
    // The index orders by whole seconds: the second note has to be visibly newer.
    await page.waitForTimeout(1200);
    const second = await postNote(request, author, `${word} にばんめ`);
    await expect
      .poll(async () => (await apiSearch(request, word, me, { kind: "SEARCH_KIND_NOTE" })).length, {
        timeout: 30_000,
      })
      .toBe(2);

    await page.goto(`/search?q=${word}&kind=notes`);
    await expect(hits(page)).toHaveCount(2);
    await page.getByRole("button", { name: /並び順/ }).click();
    await page.getByRole("menuitem", { name: "新しい順" }).click();
    await expect(page).toHaveURL((url) => url.searchParams.get("sort") === "newest");
    await expect(hits(page).first()).toHaveAttribute("href", `/notes/${second}`);
    await expect(hits(page).last()).toHaveAttribute("href", `/notes/${first}`);
  });

  test("a long list of results keeps loading as it is scrolled", async ({ page, request, me }) => {
    const author = await createAccount(request, "prolific");
    const word = unique("takusan");
    const rules = await rpc<{ version: number }>(request, `${NOTES}/GetNoteRules`, {}, author);
    await rpc(request, `${NOTES}/AgreeToNoteRules`, { version: rules.version }, author);
    for (let index = 1; index <= 24; index++)
      await rpc(
        request,
        `${NOTES}/CreateNote`,
        { text: `${word} その${index}`, visibility: "VISIBILITY_PUBLIC" },
        author,
      );
    const total = async () =>
      Number(
        (
          await rpc<{ total?: string }>(
            request,
            `${SEARCH}/Search`,
            { query: word, kind: "SEARCH_KIND_NOTE", skipHistory: true },
            me,
          )
        ).total ?? 0,
      );
    await expect.poll(total, { timeout: 30_000 }).toBe(24);

    await page.goto(`/search?q=${word}&kind=notes`);
    // A page is 20 results; the rest arrives when the end of the list comes near.
    await expect(hits(page).first()).toBeVisible();
    await hits(page).last().scrollIntoViewIfNeeded();
    await expect(hits(page)).toHaveCount(24);
    await expect(page.getByText("約 24 件")).toBeVisible();
  });

  test("a stamp pack from the store is found and opens its page", async ({ page, request, me }) => {
    const store = await rpc<{ packs?: { name: string; shortId: string }[] }>(
      request,
      "tokuly.stamp.v1.StampStoreService/ListStorePacks",
      { page: { limit: 1 } },
      me,
    );
    const pack = store.packs?.[0];
    test.skip(!pack, "the store has no published pack in this environment");
    if (!pack) return;
    await waitUntilFound(request, pack.name, me, "SEARCH_KIND_STAMP_PACK");

    await page.goto(`/search?q=${encodeURIComponent(pack.name)}&kind=stamps`);
    const card = hits(page).filter({ hasText: pack.name }).first();
    await expect(card).toHaveAttribute("href", `/stamps/${pack.shortId}`);
    await card.click();
    await expect(page).toHaveURL(new RegExp(`/stamps/${pack.shortId}$`));
    await expect(page.getByRole("heading", { level: 1, name: pack.name })).toBeVisible();
  });

  test("nothing found is not a dead end", async ({ page, me: _me }) => {
    const nonsense = unique("qqxzv");
    await page.goto(`/search?q=${nonsense}`);
    const empty = page.getByTestId("search-empty");
    await expect(empty).toContainText(`「${nonsense}」は見つからなかった`);
    await expect(empty.getByRole("link", { name: "ともだちをさがす" })).toHaveAttribute(
      "href",
      "/friends",
    );
    await empty.getByRole("link", { name: "さがすトップへ" }).click();
    await expect(page).toHaveURL(/\/search$/);
    await expect(page.getByRole("region", { name: "トレンド" })).toBeVisible();
  });
});

test.describe("typeahead", () => {
  test("answers while typing; arrow keys and Enter jump straight to a person", async ({
    page,
    request,
    me,
  }) => {
    const name = unique("Quib");
    const person = await createAccount(request, name);
    await waitUntilFound(request, name, me);

    await page.goto("/search");
    await searchBox(page).fill(name.slice(0, 6));
    const option = page.getByRole("option", { name: new RegExp(name) });
    await arrowTo(page, option);
    // The field keeps the focus and names the active option.
    await expect(searchBox(page)).toBeFocused();
    await expect(searchBox(page)).toHaveAttribute(
      "aria-activedescendant",
      (await option.getAttribute("id")) as string,
    );
    await page.keyboard.press("Enter");
    await expect(page).toHaveURL(new RegExp(`/u/${person.handle}$`));
  });

  test("Escape closes the list, and Enter without a selection searches for the text", async ({
    page,
    request,
    me,
  }) => {
    const name = unique("Vexo");
    await createAccount(request, name);
    await waitUntilFound(request, name, me);

    await page.goto("/search");
    await searchBox(page).fill(name);
    await expect(page.getByRole("listbox", { name: "検索の候補" })).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(page.getByRole("listbox", { name: "検索の候補" })).toHaveCount(0);

    await searchBox(page).press("Enter");
    await expect(page).toHaveURL((url) => url.searchParams.get("q") === name);
    await expect(hits(page).filter({ hasText: name })).toBeVisible();
  });

  test("the last line of the list searches for what was typed", async ({ page, me: _me }) => {
    const text = unique("frip");
    await page.goto("/search");
    await searchBox(page).fill(text);
    await page.getByRole("option", { name: `「${text}」で検索` }).click();
    await expect(page).toHaveURL((url) => url.searchParams.get("q") === text);
  });
});

test.describe("recent searches", () => {
  test("searches are remembered, offered again, and can be forgotten one by one or all at once", async ({
    page,
    request,
    me,
  }) => {
    const first = unique("alfa");
    const second = unique("beta");
    await page.goto("/search");
    await searchFor(page, first);
    await searchFor(page, second);

    await page.goto("/search");
    const recent = page.getByRole("region", { name: "最近の検索" });
    await expect(recent.getByRole("link", { name: first })).toBeVisible();
    await expect(recent.getByRole("link", { name: second })).toBeVisible();

    // A recent search runs again with one tap.
    await recent.getByRole("link", { name: first }).click();
    await expect(page).toHaveURL((url) => url.searchParams.get("q") === first);
    await page.goBack();

    // While typing, the person's own searches complete the input.
    await searchBox(page).fill(second.slice(0, 5));
    await expect(page.getByRole("option", { name: new RegExp(`^${second}`) })).toBeVisible();
    await page.keyboard.press("Escape");
    await searchBox(page).fill("");

    await recent.getByRole("button", { name: `「${first}」を履歴から消す` }).click();
    await expect(recent.getByRole("link", { name: first })).toHaveCount(0);
    await expect(recent.getByRole("link", { name: second })).toBeVisible();

    await recent.getByRole("button", { name: "検索履歴の設定" }).click();
    await page.getByRole("menuitem", { name: "ぜんぶ消す" }).click();
    await page.getByRole("dialog").getByRole("button", { name: "ぜんぶ消す" }).click();
    await expect(recent).toContainText("検索すると、ここに履歴がのこるよ");
    const remembered = await rpc<{ searches?: unknown[] }>(
      request,
      `${SEARCH}/ListRecentSearches`,
      {},
      me,
    );
    expect(remembered.searches ?? []).toHaveLength(0);
  });

  test("a paused history remembers nothing until it is resumed", async ({ page, request, me }) => {
    await page.goto("/search");
    const recent = page.getByRole("region", { name: "最近の検索" });
    await recent.getByRole("button", { name: "検索履歴の設定" }).click();
    await page.getByRole("menuitem", { name: "履歴を一時停止する" }).click();
    await expect(recent.getByText("一時停止中", { exact: true })).toBeVisible();

    const query = unique("gama");
    await searchFor(page, query);
    await expect(page.getByTestId("search-empty")).toBeVisible();
    await page.goto("/search");
    await expect(page.getByRole("region", { name: "最近の検索" })).toContainText(
      "履歴は一時停止中",
    );
    const paused = await rpc<{ searches?: unknown[]; historyPaused?: boolean }>(
      request,
      `${SEARCH}/ListRecentSearches`,
      {},
      me,
    );
    expect(paused).toMatchObject({ historyPaused: true });
    expect(paused.searches ?? []).toHaveLength(0);

    await page
      .getByRole("region", { name: "最近の検索" })
      .getByRole("button", { name: "検索履歴の設定" })
      .click();
    await page.getByRole("menuitem", { name: "履歴を再開する" }).click();
    await expect(
      page.getByRole("region", { name: "最近の検索" }).getByText("一時停止中", { exact: true }),
    ).toHaveCount(0);
    await searchFor(page, query);
    await page.goto("/search");
    await expect(
      page.getByRole("region", { name: "最近の検索" }).getByRole("link", { name: query }),
    ).toBeVisible();
  });

  test("a result page opened from a link is not remembered", async ({ page, request, me }) => {
    const shared = unique("delt");
    await page.goto(`/search?q=${shared}`);
    await expect(page.getByTestId("search-empty")).toBeVisible();
    const remembered = await rpc<{ searches?: unknown[] }>(
      request,
      `${SEARCH}/ListRecentSearches`,
      {},
      me,
    );
    expect(remembered.searches ?? []).toHaveLength(0);
  });
});

test.describe("explore", () => {
  test("before a query the page offers people you may know, who can be added right there", async ({
    page,
    request,
    me,
  }) => {
    // A friend of a friend is a suggestion.
    const friend = await createAccount(request, "friend");
    const friendOfFriend = await createAccount(request, unique("Maybe"));
    await befriend(request, me, friend);
    await befriend(request, friend, friendOfFriend);

    await page.goto("/search");
    const people = page.getByRole("region", { name: "もしかして、友達!?" });
    const card = people.getByRole("listitem").filter({ hasText: friendOfFriend.displayName });
    await expect(card).toBeVisible();
    await expect(card.getByRole("link")).toHaveAttribute("href", `/u/${friendOfFriend.handle}`);
    await card.getByRole("button", { name: "追加" }).click();
    await expect(card).toContainText("リクエスト済み");
    const outgoing = await rpc<{ requests?: { to?: { id: string } }[] }>(
      request,
      "tokuly.social.v1.FriendService/ListFriendRequests",
      { direction: "FRIEND_REQUEST_DIRECTION_OUTGOING" },
      me,
    );
    expect((outgoing.requests ?? []).map((item) => item.to?.id)).toContain(friendOfFriend.userId);
  });
});

test.describe("quick search", () => {
  test("opens from anywhere with the keyboard and jumps to a person", async ({
    page,
    request,
    me,
  }) => {
    const name = unique("Palo");
    const person = await createAccount(request, name);
    await waitUntilFound(request, name, me);

    await page.goto("/me");
    await expect(page.getByRole("heading", { name: "じぶん" })).toBeVisible();
    await page.keyboard.press("ControlOrMeta+k");
    const palette = page.getByRole("dialog", { name: "クイック検索" });
    const field = palette.getByRole("combobox", { name: "クイック検索" });
    await expect(field).toBeFocused();

    await field.fill(name.slice(0, 6));
    await arrowTo(page, palette.getByRole("option", { name: new RegExp(name) }));
    await page.keyboard.press("Enter");
    await expect(page).toHaveURL(new RegExp(`/u/${person.handle}$`));
    await expect(palette).toHaveCount(0);
  });

  test("a shortcut whose code cannot load leaves the app up, and works once the device is back online", async ({
    page,
    me: _me,
  }) => {
    // The shortcut's code fails to arrive, as when the connection drops right as the app opens.
    const letThrough = await failToLoad(page, "background.tsx", "search");

    await page.goto("/me");
    await expect(page.getByRole("heading", { name: "じぶん" })).toBeVisible();
    await expect(page.getByRole("heading", { name: "うまくいきませんでした" })).toHaveCount(0);

    await letThrough();
    await backOnline(page);
    const palette = page.getByRole("dialog", { name: "クイック検索" });
    await expect(async () => {
      await page.keyboard.press("ControlOrMeta+k");
      await expect(palette).toBeVisible({ timeout: 5_000 });
    }).toPass();
  });

  test("'/' opens it too, Enter goes to the result page, Escape closes it", async ({
    page,
    me: _me,
  }) => {
    await page.goto("/me");
    await expect(page.getByRole("heading", { name: "じぶん" })).toBeVisible();
    await page.keyboard.press("/");
    const palette = page.getByRole("dialog", { name: "クイック検索" });
    await expect(palette).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(palette).toHaveCount(0);

    await page.keyboard.press("/");
    const query = unique("slsh");
    await palette.getByRole("combobox").fill(query);
    await page.keyboard.press("Enter");
    await expect(page).toHaveURL(
      (url) => url.pathname === "/search" && url.searchParams.get("q") === query,
    );
    await expect(palette).toHaveCount(0);
    // On the search page the shortcut puts the cursor into the page's own field.
    await page.keyboard.press("Escape");
    await page.getByRole("heading", { name: "さがす" }).click();
    await page.keyboard.press("/");
    await expect(searchBox(page)).toBeFocused();
    await expect(palette).toHaveCount(0);
  });
});

test.describe("staff", () => {
  /** A second browser session, signed in as the deployment's admin, next to the test's own. */
  async function adminSession(
    browser: Browser,
    request: APIRequestContext,
  ): Promise<{ staff: Page; close(): Promise<void> }> {
    const { baseURL, locale, timezoneId, viewport, hasTouch } = test.info().project.use;
    const context = await browser.newContext({ baseURL, locale, timezoneId, viewport, hasTouch });
    const staff = await context.newPage();
    await signInAs(staff, await adminAccount(request));
    return { staff, close: () => context.close() };
  }

  test("an account without the admin role is told who can open the page", async ({
    page,
    me: _me,
  }) => {
    await page.goto("/admin/search");
    await expect(page.getByText("ここはスタッフ専用だよ")).toBeVisible();
  });

  test("index stats are listed for every kind, and one can be rebuilt", async ({
    page,
    request,
  }) => {
    await signInAs(page, await adminAccount(request));
    await page.goto("/admin/search");
    await expect(
      page.getByRole("navigation", { name: "管理コンソール" }).getByRole("link", { name: "検索" }),
    ).toHaveAttribute("aria-current", "page");
    await expect(page.getByText("稼働中")).toBeVisible();
    await expect(page.getByTestId("index-row")).toHaveCount(17);
    const people = page.getByTestId("index-row").filter({ has: page.getByText(/^[a-z]+_user$/) });
    await expect(people).toContainText("ひと");
    await expect(people.getByRole("progressbar")).toBeVisible();

    // Clips have no domain behind them yet: rebuilding that index disturbs nobody.
    await page.getByRole("button", { name: "クリップを再インデックス" }).click();
    await page.getByRole("dialog").getByRole("button", { name: "再インデックス" }).click();
    await expect(page.getByText(/件を書き込み待ちに入れたよ/)).toBeVisible();
    await expect(page.getByTestId("index-row").filter({ hasText: "クリップ" })).toContainText(
      "同期済み",
    );
  });

  test("a document removed by staff is no longer found", async ({ page, request, me, browser }) => {
    const name = unique("Gone");
    const person = await createAccount(request, name);
    await waitUntilFound(request, name, me);
    await page.goto(`/search?q=${name}`);
    await expect(hits(page).filter({ hasText: `@${person.handle}` })).toBeVisible();

    const { staff, close } = await adminSession(browser, request);
    await staff.goto("/admin/search?tab=documents");
    await staff.getByRole("searchbox", { name: "外したいドキュメントを検索" }).fill(name);
    const row = staff.getByTestId("document-row").filter({ hasText: name });
    await expect(row).toContainText(person.userId);
    await row.getByRole("button", { name: `「${name}」をインデックスから外す` }).click();
    await staff.getByRole("dialog").getByRole("button", { name: "インデックスから外す" }).click();
    await expect(staff.getByText("インデックスから外したよ")).toBeVisible();
    await close();

    await expect
      .poll(async () => (await apiSearch(request, name, me)).length, { timeout: 20_000 })
      .toBe(0);
    await page.reload();
    await expect(page.getByTestId("search-empty")).toBeVisible();
  });

  test("a blocked term takes a popular query out of the suggestions", async ({
    page,
    request,
    me,
    browser,
  }) => {
    // A query becomes a popular completion once enough different people searched it: that
    // takes five searchers and the next computation of the trends (up to a minute).
    test.slow();
    const name = unique("Trendo");
    await createAccount(request, name);
    await waitUntilFound(request, name, me);
    const searchers = await Promise.all(
      Array.from({ length: 5 }, (_, index) => createAccount(request, `fan${index}`)),
    );
    for (const searcher of searchers)
      await apiSearch(request, name, searcher, { skipHistory: false });
    const completions = async () =>
      (
        await rpc<{ completions?: { query: string }[] }>(
          request,
          `${SEARCH}/Suggest`,
          { query: name.slice(0, 8) },
          me,
        )
      ).completions ?? [];
    await expect
      .poll(async () => (await completions()).map((item) => item.query.toLowerCase()), {
        timeout: 100_000,
        intervals: [3000],
        message: "the query becomes a popular completion",
      })
      .toContain(name.toLowerCase());

    await page.goto("/search");
    await searchBox(page).fill(name.slice(0, 8));
    // Popular completions are stored in their normalized (lowercase) form; the person's card keeps its capital.
    const completion = page.getByRole("option", { name: new RegExp(`^${name.toLowerCase()}`) });
    await expect(completion).toBeVisible();
    await page.keyboard.press("Escape");

    const { staff, close } = await adminSession(browser, request);
    await staff.goto("/admin/search?tab=blocked");
    await staff.getByRole("textbox", { name: "ことば" }).fill(name);
    await staff.getByRole("textbox", { name: "メモ" }).fill("e2e");
    await staff.getByRole("button", { name: "ブロックする" }).click();
    const blocked = staff.getByTestId("blocked-term").filter({ hasText: name.toLowerCase() });
    await expect(blocked).toContainText("e2e");

    // The person still finds the profile itself; only the suggestion to search for it is gone.
    await expect.poll(async () => (await completions()).length, { timeout: 20_000 }).toBe(0);
    await searchBox(page).fill(name.slice(0, 7));
    await expect(page.getByRole("option", { name: new RegExp(name) })).toBeVisible();
    await expect(completion).toHaveCount(0);

    // Unblocking is one tap, and what the test added does not stay behind.
    await blocked
      .getByRole("button", { name: `「${name.toLowerCase()}」のブロックを外す` })
      .click();
    await expect(blocked).toHaveCount(0);
    await close();
  });

  test("synonym groups can be added, changed and deleted", async ({ page, request }) => {
    const admin = await adminAccount(request);
    await signInAs(page, admin);
    const [one, two, three] = [unique("syna"), unique("synb"), unique("sync")];
    await page.goto("/admin/search?tab=synonyms");

    await page.getByRole("button", { name: "グループを追加" }).click();
    const dialog = page.getByRole("dialog");
    await dialog.getByRole("textbox", { name: "ことば" }).fill(one);
    // One word is not a group.
    await expect(dialog.getByRole("button", { name: "保存する" })).toBeDisabled();
    await dialog.getByRole("textbox", { name: "ことば" }).fill(`${one}, ${two}`);
    await dialog.getByRole("button", { name: "保存する" }).click();
    const group = page.getByTestId("synonym-group").filter({ hasText: one });
    await expect(group).toContainText(two);

    await group.getByRole("button", { name: /を編集$/ }).click();
    await page
      .getByRole("dialog")
      .getByRole("textbox", { name: "ことば" })
      .fill(`${one}\n${two}\n${three}`);
    await page.getByRole("dialog").getByRole("button", { name: "保存する" }).click();
    await expect(group).toContainText(three);

    await group.getByRole("button", { name: /を削除$/ }).click();
    await page.getByRole("dialog").getByRole("button", { name: "削除する" }).click();
    await expect(group).toHaveCount(0);
    const left = await rpc<{ groups?: { words: string[] }[] }>(
      request,
      `${SEARCH_ADMIN}/ListSynonymGroups`,
      {},
      admin,
    );
    expect((left.groups ?? []).some((item) => item.words.includes(one))).toBe(false);
  });
});
