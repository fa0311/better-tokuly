import type { APIRequestContext, Browser, Page, TestInfo } from "@playwright/test";
import { type Account, createAccount, expect, RpcError, rpc, signInAs, test } from "./support";

const COMMUNITY = "tokuly.community.v1.CommunityService";
const BOARD = "tokuly.community.v1.CommunityBoardService";
const EVENTS = "tokuly.community.v1.CommunityEventService";
const SEARCH = "tokuly.search.v1.SearchService";

interface Community {
  id: string;
  handle: string;
  name: string;
}
interface Thread {
  id: string;
  shortId: string;
  title: string;
  poll?: { options: { id: string; text: string }[] };
}

const unique = () =>
  `${Date.now().toString(36).slice(-5)}${Math.random().toString(36).slice(2, 6)}`;

/** A community owned by `owner`, created through the API. */
async function createCommunity(
  request: APIRequestContext,
  owner: Account,
  options: { visibility?: "PUBLIC" | "RESTRICTED" | "HIDDEN"; joinQuestions?: string[] } = {},
): Promise<Community> {
  const suffix = unique();
  const { community } = await rpc<{ community: Community }>(
    request,
    `${COMMUNITY}/CreateCommunity`,
    {
      name: `テスト部 ${suffix}`,
      handle: `e2e_${suffix}`,
      description: "ブラウザテスト用のコミュニティ",
      category: "COMMUNITY_CATEGORY_CLUB",
      visibility: `COMMUNITY_VISIBILITY_${options.visibility ?? "PUBLIC"}`,
      joinQuestions: options.joinQuestions ?? [],
    },
    owner,
  );
  return community;
}

async function inviteCode(
  request: APIRequestContext,
  community: Community,
  as: Account,
): Promise<string> {
  const { invite } = await rpc<{ invite: { code: string } }>(
    request,
    `${COMMUNITY}/CreateInvite`,
    { communityId: community.id },
    as,
  );
  return invite.code;
}

/** Makes `account` a member (by invite, so it works for every visibility). */
async function join(
  request: APIRequestContext,
  community: Community,
  owner: Account,
  account: Account,
): Promise<void> {
  const code = await inviteCode(request, community, owner);
  await rpc(request, `${COMMUNITY}/JoinCommunity`, { inviteCode: code }, account);
}

async function firstChannelId(
  request: APIRequestContext,
  community: Community,
  as: Account,
): Promise<string> {
  const { channels } = await rpc<{ channels: { id: string }[] }>(
    request,
    `${BOARD}/ListChannels`,
    { communityId: community.id },
    as,
  );
  return channels[0]?.id ?? "";
}

async function createThread(
  request: APIRequestContext,
  channelId: string,
  as: Account,
  title: string,
  body = "本文です",
): Promise<Thread> {
  const { thread } = await rpc<{ thread: Thread }>(
    request,
    `${BOARD}/CreateThread`,
    { channelId, title, body },
    as,
  );
  return thread;
}

/** A second person's browser, signed in, at the size of the running project. */
async function browserOf(browser: Browser, testInfo: TestInfo, account: Account): Promise<Page> {
  const context = await browser.newContext({
    baseURL: testInfo.project.use.baseURL,
    locale: "ja-JP",
    timezoneId: testInfo.project.use.timezoneId,
    viewport: testInfo.project.use.viewport,
    hasTouch: testInfo.project.use.hasTouch,
  });
  const page = await context.newPage();
  await signInAs(page, account);
  return page;
}

test("the hub leads to communities, and an empty list points at the directory", async ({
  page,
  me: _me,
}) => {
  await page.goto("/me");
  await page.getByRole("main").getByRole("link", { name: "コミュニティ" }).click();
  await expect(page).toHaveURL(/\/communities$/);
  await expect(page.getByText("まだどこにも入ってないよ")).toBeVisible();
  await page.getByRole("main").getByRole("button", { name: "さがす" }).click();
  await expect(page).toHaveURL(/tab=discover/);
  await expect(page.getByPlaceholder("なまえ、キーワードでさがす")).toBeVisible();
});

test("create a community, post a thread with a poll; a friend joins by invite code, votes and replies", async ({
  page,
  me,
  request,
  browser,
}, testInfo) => {
  const handle = `e2e_${unique()}`;
  await page.goto("/communities/new");
  await page.getByLabel("なまえ", { exact: true }).fill("放課後ねこ部");
  await page.getByLabel("ハンドル").fill(handle);
  await expect(page.getByText("このハンドル、使えるよ！")).toBeVisible();
  await page.getByLabel("しょうかい").fill("ねこが好きな人あつまれ！");
  await page.getByRole("button", { name: "部活・サークル" }).click();
  await page.getByLabel("タグ", { exact: true }).fill("#ねこ 写真");
  await page.getByRole("button", { name: "コミュニティをつくる！" }).click();

  await expect(page).toHaveURL(new RegExp(`/g/${handle}$`));
  await expect(page.getByRole("heading", { name: "放課後ねこ部", level: 2 })).toBeVisible();
  await expect(page.getByText("オーナー").first()).toBeVisible();
  await expect(page.getByText("#ねこ")).toBeVisible();

  // A thread with a poll.
  test.slow();
  await page.getByRole("button", { name: "スレッドを立てる" }).first().click();
  const composer = page.getByRole("dialog");
  await composer.getByLabel("タイトル").fill("次の集まり、いつにする？");
  await composer.getByLabel("本文").fill("みんなの都合を教えて！");
  await composer.getByRole("button", { name: "アンケートをつける" }).click();
  await composer.getByLabel("選択肢1").fill("土曜");
  await composer.getByLabel("選択肢2").fill("日曜");
  await composer.getByRole("button", { name: "投稿する" }).click();

  await expect(page).toHaveURL(new RegExp(`/g/${handle}/t/[\\w-]{11}$`));
  await expect(page.getByRole("heading", { name: "次の集まり、いつにする？" })).toBeVisible();
  const poll = page.getByRole("region", { name: "アンケート" });
  await expect(poll.getByText("土曜")).toBeVisible();
  await expect(poll.getByText("0人が投票")).toBeVisible();
  const threadUrl = new URL(page.url()).pathname;

  // The friend opens the invite, joins, votes and replies.
  const community = { id: handle, handle, name: "放課後ねこ部" };
  const code = await inviteCode(request, community, me);
  const friend = await createAccount(request, "tomo");
  const theirs = await browserOf(browser, testInfo, friend);
  await theirs.goto(`/join/${code}`);
  await expect(theirs.getByRole("heading", { name: "放課後ねこ部" })).toBeVisible();
  await expect(theirs.getByText("meさんからの招待")).toBeVisible();
  await theirs.getByRole("button", { name: "参加する！" }).click();
  await expect(theirs).toHaveURL(new RegExp(`/g/${handle}$`));
  await expect(theirs.getByText("ようこそ！参加したよ")).toBeVisible();

  await theirs.getByRole("link", { name: "次の集まり、いつにする？" }).click();
  await expect(theirs).toHaveURL(new RegExp(`${threadUrl}$`));
  const theirPoll = theirs.getByRole("region", { name: "アンケート" });
  await expect(theirPoll.getByText("投票すると結果が見られるよ")).toBeHidden();
  await theirPoll.getByRole("button", { name: "日曜" }).click();
  await theirPoll.getByRole("button", { name: "投票する" }).click();
  await expect(theirPoll.getByText("100%（1票）")).toBeVisible();
  await expect(theirPoll.getByText("1人が投票")).toBeVisible();

  await theirs.getByLabel("返信", { exact: true }).fill("日曜なら行ける！");
  await theirs.getByRole("button", { name: "返信する" }).click();
  await expect(
    theirs.getByRole("main").getByRole("listitem").filter({ hasText: "日曜なら行ける！" }),
  ).toBeVisible();

  // The author sees both without reloading: pushes name IDs, the page fetches what it may read.
  await expect(
    page.getByRole("main").getByRole("listitem").filter({ hasText: "日曜なら行ける！" }),
  ).toBeVisible();
  await expect(poll.getByText("1人が投票")).toBeVisible();
  await theirs.context().close();
});

test("an invite link shows the community to a visitor without an account and brings them back after sign-in", async ({
  browser,
  request,
}, testInfo) => {
  const owner = await createAccount(request, "owner");
  const community = await createCommunity(request, owner, { visibility: "HIDDEN" });
  const code = await inviteCode(request, community, owner);
  const context = await browser.newContext({
    baseURL: testInfo.project.use.baseURL,
    locale: "ja-JP",
    viewport: testInfo.project.use.viewport,
  });
  const visitor = await context.newPage();
  await visitor.goto(`/join/${code}`);
  await expect(visitor.getByRole("heading", { name: community.name })).toBeVisible();
  await expect(visitor.getByText("招待のみ")).toBeVisible();
  const signIn = visitor.getByRole("link", { name: "ログインして参加" });
  await expect(signIn).toHaveAttribute("href", new RegExp(`redirect=.*join.*${code}`));

  await visitor.goto("/join/nosuchcode0");
  await expect(visitor.getByText("この招待は使えないみたい")).toBeVisible();
  await context.close();
});

test("a restricted community shows its front door; a join request is answered and approved", async ({
  page,
  me,
  request,
  browser,
}, testInfo) => {
  const owner = await createAccount(request, "owner");
  const community = await createCommunity(request, owner, {
    visibility: "RESTRICTED",
    joinQuestions: ["何年何組？"],
  });
  const channelId = await firstChannelId(request, community, owner);
  await createThread(request, channelId, owner, "メンバーだけのひみつ");

  await page.goto(`/g/${community.handle}`);
  await expect(page.getByText("ここから先はメンバーだけ")).toBeVisible();
  await expect(page.getByText("メンバーだけのひみつ")).toBeHidden();
  await page.getByRole("button", { name: "参加リクエスト" }).click();
  const dialog = page.getByRole("dialog");
  await dialog.getByLabel("何年何組？").fill("2年B組です");
  await dialog.getByRole("button", { name: "リクエストを送る" }).click();
  await expect(page.getByText("参加リクエスト中。スタッフの返事を待ってね。")).toBeVisible();

  const staff = await browserOf(browser, testInfo, owner);
  await staff.goto(`/g/${community.handle}/manage`);
  await staff.getByRole("link", { name: /参加リクエスト/ }).click();
  const request1 = staff.getByRole("article").filter({ hasText: me.handle });
  await expect(request1.getByText("2年B組です")).toBeVisible();
  await request1.getByRole("button", { name: "承認する" }).click();
  await expect(staff.getByText("参加リクエストはないよ")).toBeVisible();

  // The requester's page opens up by itself.
  await expect(page.getByRole("link", { name: "メンバーだけのひみつ" })).toBeVisible();
  await expect(page.getByRole("button", { name: "スレッドを立てる" })).toBeVisible();
  await staff.context().close();
});

test("an anonymous channel never names the author; an uninvolved moderator can look it up with a reason", async ({
  page,
  me,
  request,
  browser,
}, testInfo) => {
  // Three browsers and a dozen API calls: more than the default budget on a busy machine.
  test.slow();
  const owner = await createAccount(request, "owner");
  const author = await createAccount(request, "himitsu");
  const community = await createCommunity(request, owner);
  // An anonymous channel takes posts once five members could have written them.
  const extras = await Promise.all(
    ["a", "b", "c"].map((name) => createAccount(request, `member${name}`)),
  );
  for (const account of [me, author, ...extras])
    await rpc(request, `${COMMUNITY}/JoinCommunity`, { communityId: community.id }, account);
  const { channel } = await rpc<{ channel: { id: string } }>(
    request,
    `${BOARD}/CreateChannel`,
    { communityId: community.id, name: "ないしょ話", emoji: "🤫", anonymous: true },
    owner,
  );
  const thread = await createThread(request, channel.id, author, "だれにも言えない話", "実は…");

  // A member reads it under a pseudonym and has no way to the author.
  await page.goto(`/g/${community.handle}/t/${thread.shortId}`);
  await expect(page.getByRole("heading", { name: "だれにも言えない話" })).toBeVisible();
  const post = page.getByRole("article").filter({ hasText: "だれにも言えない話" });
  await expect(post.getByText("匿名", { exact: true })).toBeVisible();
  await expect(post.getByText(/#[\w]{4}/)).toBeVisible();
  await expect(page.getByText(author.displayName)).toBeHidden();
  await expect(page.getByText(author.handle)).toBeHidden();
  await expect(post.getByText("あなた", { exact: true })).toBeHidden();
  await page.getByRole("button", { name: "スレッドのメニュー" }).click();
  await expect(page.getByRole("menuitem", { name: "通報する" })).toBeVisible();
  await expect(page.getByRole("menuitem", { name: "投稿者を確認する" })).toBeHidden();
  await page.keyboard.press("Escape");

  // The author alone is told the post is theirs.
  const theirs = await browserOf(browser, testInfo, author);
  await theirs.goto(`/g/${community.handle}/t/${thread.shortId}`);
  await expect(
    theirs
      .getByRole("article")
      .filter({ hasText: "だれにも言えない話" })
      .getByText("あなた", { exact: true }),
  ).toBeVisible();
  await theirs.context().close();

  // A moderator sees the same pseudonym; the name appears only after a logged lookup.
  const staff = await browserOf(browser, testInfo, owner);
  await staff.goto(`/g/${community.handle}/t/${thread.shortId}`);
  await expect(staff.getByRole("heading", { name: "だれにも言えない話" })).toBeVisible();
  await expect(staff.getByText(author.displayName)).toBeHidden();
  await staff.getByRole("button", { name: "スレッドのメニュー" }).click();
  await staff.getByRole("menuitem", { name: "投稿者を確認する" }).click();
  const lookup = staff.getByRole("dialog");
  await expect(lookup.getByRole("button", { name: "記録して確認する" })).toBeDisabled();
  await lookup.getByLabel("確認する理由（必須）").fill("通報の対応のため");
  await lookup.getByRole("button", { name: "記録して確認する" }).click();
  await expect(lookup.getByText(`@${author.handle}`)).toBeVisible();
  await lookup.getByRole("button", { name: "閉じる" }).click();
  await expect(staff.getByText(author.handle)).toBeHidden();

  // The lookup is on the log, without the answer.
  await staff.goto(`/g/${community.handle}/manage/log`);
  const entry = staff.getByRole("listitem").filter({ hasText: "匿名の投稿者を確認した" });
  await expect(entry.getByText("理由: 通報の対応のため")).toBeVisible();
  await expect(staff.getByText(author.handle)).toBeHidden();

  // Once the moderator wrote in the thread themselves, they are a party and are refused.
  await rpc(request, `${BOARD}/CreateReply`, { threadId: thread.id, body: "わたしも" }, owner);
  await staff.goto(`/g/${community.handle}/t/${thread.shortId}`);
  await staff.getByRole("button", { name: "スレッドのメニュー" }).click();
  await staff.getByRole("menuitem", { name: "投稿者を確認する" }).click();
  await staff.getByRole("dialog").getByLabel("確認する理由（必須）").fill("もう一度");
  await staff.getByRole("dialog").getByRole("button", { name: "記録して確認する" }).click();
  await expect(
    staff.getByText("自分がかかわることは、ほかのスタッフにおねがいしてね"),
  ).toBeVisible();
  await expect(staff.getByText(author.handle)).toBeHidden();
  await staff.context().close();
});

test("a moderator pins and locks a thread; a locked thread takes no replies", async ({
  page,
  me,
  request,
}) => {
  const community = await createCommunity(request, me);
  const member = await createAccount(request, "member");
  await join(request, community, me, member);
  const channelId = await firstChannelId(request, community, me);
  const older = await createThread(request, channelId, member, "ふつうのスレッド");
  await createThread(request, channelId, member, "あたらしいスレッド");

  await page.goto(`/g/${community.handle}/t/${older.shortId}`);
  await expect(page.getByLabel("返信", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "スレッドのメニュー" }).click();
  await page.getByRole("menuitem", { name: "ピン留めする" }).click();
  await expect(page.getByText("ピン留めしたよ")).toBeVisible();
  await page.getByRole("button", { name: "スレッドのメニュー" }).click();
  await page.getByRole("menuitem", { name: "ロックする" }).click();
  await expect(page.getByText("このスレッドはロック中。返信はできないよ。")).toBeVisible();
  await expect(page.getByLabel("返信", { exact: true })).toBeHidden();

  // Pinned threads lead the board, newest order or not.
  await page.goto(`/g/${community.handle}?sort=new`);
  const cards = page.getByRole("article");
  await expect(cards.first()).toContainText("ふつうのスレッド");
  await expect(cards.first().getByText("ピン留め")).toBeVisible();
  await expect(cards.first().getByText("ロック中")).toBeVisible();
  await expect(cards.nth(1)).toContainText("あたらしいスレッド");

  const refused = await rpc(
    request,
    `${BOARD}/CreateReply`,
    { threadId: older.id, body: "まだ書ける？" },
    member,
  ).catch((error) => error);
  expect(refused).toBeInstanceOf(RpcError);
  expect((refused as RpcError).reason).toBe("THREAD_LOCKED");
});

test("an event fills up, the next person is waitlisted and moves up when a place frees", async ({
  page,
  me,
  request,
}) => {
  const owner = await createAccount(request, "owner");
  const second = await createAccount(request, "second");
  const community = await createCommunity(request, owner);
  for (const account of [me, second])
    await rpc(request, `${COMMUNITY}/JoinCommunity`, { communityId: community.id }, account);
  // The organiser is going; one more fills it.
  const { event } = await rpc<{ event: { id: string } }>(
    request,
    `${EVENTS}/CreateEvent`,
    {
      communityId: community.id,
      title: "テスト前の勉強会",
      startsAt: new Date(Date.now() + 3 * 86_400_000).toISOString(),
      place: { name: "図書室" },
      capacity: 2,
    },
    owner,
  );
  await rpc(request, `${EVENTS}/Rsvp`, { eventId: event.id, answer: "RSVP_ANSWER_GOING" }, second);

  await page.goto(`/g/${community.handle}?tab=events`);
  await page.getByRole("link", { name: /テスト前の勉強会/ }).click();
  await expect(page).toHaveURL(new RegExp(`/events/${event.id}$`));
  await expect(page.getByText("2/2人")).toBeVisible();
  await expect(page.getByText("満員")).toBeVisible();
  await page.getByRole("button", { name: "キャンセル待ちする" }).click();
  await expect(page.getByText("キャンセル待ち 1番目")).toBeVisible();
  await expect(page.getByRole("button", { name: "キャンセル待ち中" })).toHaveAttribute(
    "aria-pressed",
    "true",
  );

  // A place frees: the first on the waitlist is going, without a reload.
  await rpc(
    request,
    `${EVENTS}/Rsvp`,
    { eventId: event.id, answer: "RSVP_ANSWER_NOT_GOING" },
    second,
  );
  await expect(page.getByRole("button", { name: "行く！" })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  await expect(page.getByText("キャンセル待ち 1番目")).toBeHidden();
});

test("an organiser creates an event from the calendar tab", async ({ page, me, request }) => {
  const community = await createCommunity(request, me);
  await page.goto(`/g/${community.handle}?tab=events`);
  await page.getByRole("button", { name: "イベントをつくる" }).click();
  const form = page.getByRole("dialog");
  await form.getByLabel("イベント名").fill("ねこカフェオフ会");
  await form.getByLabel("場所のなまえ").fill("駅前のねこカフェ");
  await form.getByLabel("定員").fill("5");
  await form.getByRole("button", { name: "イベントをつくる" }).click();
  await expect(page.getByText("イベントをつくったよ！")).toBeVisible();
  const card = page.getByRole("link", { name: /ねこカフェオフ会/ });
  await expect(card.getByText("駅前のねこカフェ")).toBeVisible();
  await expect(card.getByText("1/5人")).toBeVisible();
  // Tomorrow 18:00 on the account's clock, whatever the device's zone.
  await expect(card.getByText("18:00")).toBeVisible();
});

test("a ban takes a member's access away at once and says why", async ({
  page,
  me,
  request,
  browser,
}, testInfo) => {
  const owner = await createAccount(request, "owner");
  const community = await createCommunity(request, owner, { visibility: "RESTRICTED" });
  await join(request, community, owner, me);
  const channelId = await firstChannelId(request, community, owner);
  await createThread(request, channelId, owner, "メンバー限定のおはなし");

  await page.goto(`/g/${community.handle}`);
  await expect(page.getByRole("link", { name: "メンバー限定のおはなし" })).toBeVisible();

  const staff = await browserOf(browser, testInfo, owner);
  await staff.goto(`/g/${community.handle}?tab=members`);
  await staff.getByRole("button", { name: `${me.displayName}のメニュー` }).click();
  await staff.getByRole("menuitem", { name: "BANする" }).click();
  const dialog = staff.getByRole("dialog");
  await dialog.getByRole("button", { name: "BANする" }).click();
  await expect(dialog.getByText("理由を書いてね")).toBeVisible();
  await dialog.getByLabel("理由（必須）").fill("ルール違反がつづいたため");
  await dialog.getByRole("button", { name: "BANする" }).click();
  await expect(staff.getByText(`${me.displayName}をBANしたよ`)).toBeVisible();
  await expect(staff.getByRole("button", { name: `${me.displayName}のメニュー` })).toBeHidden();

  // The banned member's open page closes by itself.
  await expect(page.getByText("このコミュニティには参加できなくなっています")).toBeVisible();
  await expect(page.getByText("理由: ルール違反がつづいたため")).toBeVisible();
  await expect(page.getByText("メンバー限定のおはなし")).toBeHidden();
  await page.goto(`/g/${community.handle}?tab=members`);
  await expect(page.getByText("ここから先はメンバーだけ")).toBeVisible();

  // The ban is on the staff's list and can be lifted.
  await staff.goto(`/g/${community.handle}/manage/bans`);
  const ban = staff.getByRole("article").filter({ hasText: me.handle });
  await expect(ban.getByText("ルール違反がつづいたため")).toBeVisible();
  await ban.getByRole("button", { name: "解除する" }).click();
  await expect(staff.getByText("BANされている人はいないよ")).toBeVisible();
  await staff.context().close();
});

test("the staff adds an anonymous channel, sets slow mode and a muted word; a held post waits in the queue", async ({
  page,
  me,
  request,
}) => {
  test.slow();
  const community = await createCommunity(request, me);
  const member = await createAccount(request, "member");
  await join(request, community, me, member);

  await page.goto(`/g/${community.handle}/manage/channels`);
  await page.getByRole("button", { name: "チャンネルを追加" }).click();
  const dialog = page.getByRole("dialog");
  await dialog.getByLabel("チャンネル名").fill("ないしょ話");
  await dialog.getByRole("switch", { name: /匿名チャンネルにする/ }).click();
  await dialog.getByRole("button", { name: "追加する" }).click();
  await expect(page.getByText("ないしょ話")).toBeVisible();
  await expect(page.getByText("匿名", { exact: true })).toBeVisible();

  await page.goto(`/g/${community.handle}/manage/safety`);
  await page.getByLabel("ことばを入力").fill("ばか");
  await page.getByRole("button", { name: "追加", exact: true }).click();
  await expect(page.getByRole("button", { name: "「ばか」を消す" })).toBeVisible();

  // Slow mode on the first channel: members see how long they wait between posts.
  await page.locator(".rounded-card").filter({ hasText: "雑談" }).getByRole("combobox").click();
  await page.getByRole("option", { name: "30秒" }).click();
  await expect(page.getByText("保存したよ！").first()).toBeVisible();
  await page.goto(`/g/${community.handle}`);
  await page.getByRole("button", { name: /雑談/ }).click();
  await expect(page.getByText("スローモード: 30秒に1回投稿できるよ")).toBeVisible();

  // A member's post with the muted word is held: only they and the staff see it.
  const channelId = await firstChannelId(request, community, me);
  const held = await createThread(request, channelId, member, "ひどいことば", "ばかって言われた");
  await page.goto(`/g/${community.handle}/manage/queue`);
  const item = page.getByRole("article").filter({ hasText: "ひどいことば" });
  await expect(item.getByText("NGワード")).toBeVisible();
  await item.getByRole("button", { name: "もどす" }).click();
  await expect(page.getByText("確認待ちの投稿はないよ")).toBeVisible();
  await page.goto(`/g/${community.handle}/t/${held.shortId}`);
  await expect(page.getByRole("heading", { name: "ひどいことば" })).toBeVisible();
  await expect(page.getByText("確認待ち")).toBeHidden();
});

test("search results for a community and its thread land on the community's pages", async ({
  page,
  me,
  request,
}) => {
  const community = await createCommunity(request, me);
  const channelId = await firstChannelId(request, community, me);
  const word = `kensaku${unique()}`;
  const thread = await createThread(request, channelId, me, `${word} のスレッド`);

  // The address search builds for a thread hit: the short ID under `/g/<handle>/t/`.
  await page.goto(`/g/${community.handle}/t/${thread.shortId}`);
  await expect(page.getByRole("heading", { name: `${word} のスレッド` })).toBeVisible();
  // The UUID works as well.
  await page.goto(`/g/${community.handle}/t/${thread.id}`);
  await expect(page.getByRole("heading", { name: `${word} のスレッド` })).toBeVisible();

  // Through the search page itself, once the index has caught up.
  await expect(async () => {
    const found = await rpc<{ hits?: { id: string }[] }>(
      request,
      `${SEARCH}/Search`,
      { query: word },
      me,
    );
    expect(JSON.stringify(found)).toContain(thread.id);
  }).toPass({ timeout: 30_000 });
  await page.goto(`/search?q=${word}`);
  const hit = page.getByRole("link", { name: new RegExp(word) }).first();
  await expect(hit).toHaveAttribute(
    "href",
    new RegExp(`/g/${community.handle}/t/${thread.shortId}$`),
  );
});

test("a reply notification opens the thread at that reply, even past the first page", async ({
  page,
  me,
  request,
}) => {
  test.slow();
  const member = await createAccount(request, "member");
  const community = await createCommunity(request, me);
  await join(request, community, me, member);
  const thread = await createThread(
    request,
    await firstChannelId(request, community, me),
    me,
    `ながいスレッド ${unique()}`,
  );
  // More replies than the first page shows (30), then the one that is linked.
  for (let index = 1; index <= 34; index++)
    await rpc(request, `${BOARD}/CreateReply`, { threadId: thread.id, body: `メモ ${index}` }, me);
  const answer = `とどいたよ ${unique()}`;
  const { reply } = await rpc<{ reply: { id: string } }>(
    request,
    `${BOARD}/CreateReply`,
    { threadId: thread.id, body: answer },
    member,
  );

  // The thread's author is told; the server's link points at the reply.
  let entry: { title: string; path: string } | undefined;
  await expect
    .poll(
      async () => {
        const { notifications = [] } = await rpc<{
          notifications?: { kind: string; title: string; path: string }[];
        }>(request, "tokuly.notify.v1.InboxService/ListNotifications", {}, me);
        entry = notifications.find((each) => each.kind === "community.reply");
        return entry?.path ?? "";
      },
      { timeout: 20_000 },
    )
    .toContain(`reply=${reply.id}`);

  // From the inbox: the community's page (not a video channel), scrolled to the reply, highlighted.
  await page.goto("/notifications");
  await page
    .getByRole("listitem")
    .filter({ hasText: entry?.title ?? "" })
    .first()
    .getByRole("link")
    .click();
  await expect(page).toHaveURL(
    new RegExp(`/g/${community.handle}/t/${thread.shortId}\\?reply=${reply.id}$`),
  );
  const focused = page.locator('[data-testid="community-reply"][data-focused]');
  await expect(focused).toHaveCount(1);
  await expect(focused).toContainText(answer);
  await expect(focused).toBeInViewport();
});
