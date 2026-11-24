import { befriend, createAccount, expect, rpc, signInAs, test } from "./support";

const FOLLOW = "tokuly.social.v1.FollowService";

test("follow an open account: the button turns into フォロー中 and the numbers count it", async ({
  page,
  request,
  me,
}) => {
  const aoi = await createAccount(request, "Aoi");

  await page.goto(`/u/${aoi.handle}`);
  await expect(page.getByTestId("profile-followers")).toContainText("0");
  await page.getByRole("button", { name: "フォローする" }).click();
  await expect(page.getByText(`${aoi.displayName}さんをフォローしたよ!`)).toBeVisible();
  await expect(page.getByRole("button", { name: /フォロー中/ })).toBeVisible();
  await expect(page.getByTestId("profile-followers")).toContainText("1");

  // Aoi's followers list has me in it; Aoi sees "フォローされています" and a follow-back on me.
  await page.getByTestId("profile-followers").click();
  await expect(page).toHaveURL(new RegExp(`/u/${aoi.handle}/followers$`));
  await expect(page.getByTestId("follow-row").filter({ hasText: me.displayName })).toBeVisible();

  const summary = await rpc<{ summary: { relation?: { followsYou?: boolean } } }>(
    request,
    `${FOLLOW}/GetFollowSummary`,
    { userId: me.userId },
    aoi,
  );
  expect(summary.summary.relation?.followsYou).toBe(true);

  // The sheet behind フォロー中: mute for a day, then unfollow.
  await page.goto(`/u/${aoi.handle}`);
  await page.getByRole("button", { name: /フォロー中/ }).click();
  const sheet = page.getByRole("dialog");
  await sheet.getByRole("button", { name: "1日" }).click();
  await expect(sheet.getByText(/までミュート中/)).toBeVisible();
  await sheet.getByRole("button", { name: "フォローをやめる" }).click();
  await expect(page.getByRole("button", { name: "フォローする" })).toBeVisible();
  await expect(page.getByTestId("profile-followers")).toContainText("0");
});

test("a locked account: lists are private, a request waits and is approved from the requests page", async ({
  page,
  request,
  me,
}) => {
  const ren = await createAccount(request, "Ren");
  await rpc(request, `${FOLLOW}/UpdateFollowSettings`, { settings: { locked: true } }, me);

  // Ren cannot read my lists before following.
  await expect(
    rpc(
      request,
      `${FOLLOW}/ListFollows`,
      { userId: me.userId, list: "FOLLOW_LIST_FOLLOWERS" },
      ren,
    ),
  ).rejects.toMatchObject({ reason: "FOLLOW_LIST_PRIVATE" });
  const asked = await rpc<{ state: string }>(
    request,
    `${FOLLOW}/Follow`,
    { userId: me.userId },
    ren,
  );
  expect(asked.state).toBe("FOLLOW_STATE_REQUESTED");

  await page.goto("/friends/follow-requests");
  const row = page.getByTestId("follow-request").filter({ hasText: ren.displayName });
  await expect(row).toBeVisible();
  await row.getByRole("button", { name: "承認" }).click();
  await expect(page.getByText("承認したよ!")).toBeVisible();
  await expect(row).toBeHidden();

  await page.goto(`/u/${me.handle}/followers`);
  await expect(page.getByTestId("follow-row").filter({ hasText: ren.displayName })).toBeVisible();
});

test("someone else's locked lists show the friendly locked state, and following sends a request", async ({
  page,
  request,
  me: _me,
}) => {
  const sora = await createAccount(request, "Sora");
  await rpc(request, `${FOLLOW}/UpdateFollowSettings`, { settings: { locked: true } }, sora);

  await page.goto(`/u/${sora.handle}/followers`);
  await expect(page.getByText("鍵アカウントだよ🔒")).toBeVisible();

  await page.goto(`/u/${sora.handle}`);
  await page.getByRole("button", { name: "フォローする" }).click();
  await expect(page.getByText("フォローリクエストを送ったよ!")).toBeVisible();
  await expect(page.getByRole("button", { name: "リクエスト済み" })).toBeVisible();
});

test("unlocking the account approves the waiting requests", async ({ page, request, me }) => {
  const kai = await createAccount(request, "Kai");
  await rpc(request, `${FOLLOW}/UpdateFollowSettings`, { settings: { locked: true } }, me);
  await rpc(request, `${FOLLOW}/Follow`, { userId: me.userId }, kai);

  await page.goto("/settings/follows");
  const toggle = page.getByRole("switch");
  await expect(toggle).toBeChecked();
  await toggle.click();
  await page.getByRole("dialog").getByRole("button", { name: "鍵を外す" }).click();
  await expect(page.getByText("鍵を外して、1件のリクエストを承認したよ!")).toBeVisible();
  await expect(toggle).not.toBeChecked();

  const followers = await rpc<{ total?: number }>(
    request,
    `${FOLLOW}/ListFollows`,
    { list: "FOLLOW_LIST_FOLLOWERS" },
    me,
  );
  expect(followers.total ?? 0).toBe(1);
});

test("remove a follower from your own followers list", async ({ page, request, me }) => {
  const mio = await createAccount(request, "Mio");
  await rpc(request, `${FOLLOW}/Follow`, { userId: me.userId }, mio);

  await page.goto(`/u/${me.handle}/followers`);
  const row = page.getByTestId("follow-row").filter({ hasText: mio.displayName });
  await expect(row).toContainText("フォローされています");
  await row.getByRole("button", { name: "削除" }).click();
  await page.getByRole("dialog").getByRole("button", { name: "削除する" }).click();
  await expect(page.getByText("フォロワーから削除したよ")).toBeVisible();
  await expect(row).toBeHidden();

  const followers = await rpc<{ total?: number }>(
    request,
    `${FOLLOW}/ListFollows`,
    { list: "FOLLOW_LIST_FOLLOWERS" },
    me,
  );
  expect(followers.total ?? 0).toBe(0);
});

test("circles: make a list, add a friend, take them out again", async ({ page, request, me }) => {
  const yui = await createAccount(request, "Yui");
  await befriend(request, me, yui);

  await page.goto("/circles");
  await expect(page.getByText("親しい友達")).toBeVisible();
  await page.getByRole("button", { name: "つくる" }).first().click();
  const editor = page.getByRole("dialog");
  await editor.getByLabel("名前").fill("部活のなかま");
  await editor.getByRole("button", { name: "ミント" }).click();
  await editor.getByRole("button", { name: "つくる!" }).click();
  await expect(page.getByText("リストをつくったよ!")).toBeVisible();

  await page
    .getByTestId("circle-row")
    .filter({ hasText: "部活のなかま" })
    .getByRole("link")
    .click();
  await expect(page.getByText("メンバーには通知されないよ")).toBeVisible();
  await page.getByRole("button", { name: "メンバーを追加" }).first().click();
  const picker = page.getByRole("dialog");
  const candidate = picker.getByTestId("circle-candidate").filter({ hasText: yui.displayName });
  await expect(candidate).toContainText("友達");
  await candidate.getByRole("button", { name: "追加" }).click();
  await expect(candidate).toContainText("追加済み");
  await picker.getByRole("button", { name: "close" }).click();

  const member = page.getByTestId("circle-member").filter({ hasText: yui.displayName });
  await expect(member).toBeVisible();
  await expect(page.getByTestId("circle-member-count")).toHaveText("1人");
  await member.getByRole("button", { name: `${yui.displayName}さんをリストから外す` }).click();
  await expect(member).toBeHidden();
  await expect(page.getByTestId("circle-member-count")).toHaveText("0人");
});

interface SummaryResponse {
  summary: {
    relation?: { following?: string };
    prefs?: { muted?: boolean; mutedUntil?: string; notifyLevel?: string };
  };
}

interface RequestsResponse {
  requests?: { from?: { id: string }; to?: { id: string } }[];
}

test("the following sheet: post notifications on, a timed mute, and the server keeps both", async ({
  page,
  request,
  me,
}) => {
  const aoi = await createAccount(request, "Aoi");
  await rpc(request, `${FOLLOW}/Follow`, { userId: aoi.userId }, me);
  const prefs = async () =>
    (await rpc<SummaryResponse>(request, `${FOLLOW}/GetFollowSummary`, { userId: aoi.userId }, me))
      .summary.prefs;

  await page.goto(`/u/${aoi.handle}`);
  await page.getByRole("button", { name: /フォロー中/ }).click();
  const sheet = page.getByRole("dialog");
  await expect(sheet.getByText("ここでの設定は相手には知られないよ!")).toBeVisible();

  const bell = sheet.getByRole("switch");
  await expect(bell).not.toBeChecked();
  await bell.click();
  await expect(bell).toBeChecked();
  await expect.poll(async () => (await prefs())?.notifyLevel).toBe("FOLLOW_NOTIFY_LEVEL_ALL");

  await sheet.getByRole("button", { name: "1時間" }).click();
  await expect(page.getByText("ミュートしたよ", { exact: true })).toBeVisible();
  await expect(sheet.getByText(/までミュート中/)).toBeVisible();
  const muted = await prefs();
  expect(muted?.muted).toBe(true);
  const until = Date.parse(muted?.mutedUntil ?? "");
  expect(until).toBeGreaterThan(Date.now() + 50 * 60_000);
  expect(until).toBeLessThan(Date.now() + 70 * 60_000);

  // Reopened, the sheet shows what the server keeps.
  await page.reload();
  await page.getByRole("button", { name: /フォロー中/ }).click();
  await expect(sheet.getByRole("switch")).toBeChecked();
  await expect(sheet.getByText(/までミュート中/)).toBeVisible();

  await sheet.getByRole("button", { name: "ミュートを解除" }).click();
  await expect(page.getByText("ミュートを解除したよ!")).toBeVisible();
  await expect(sheet.getByRole("button", { name: "1時間" })).toBeVisible();
  expect((await prefs())?.muted ?? false).toBe(false);
});

test("declining a follow request is silent: the requester still sees リクエスト済み", async ({
  page,
  request,
  browser,
  me,
}, testInfo) => {
  const ren = await createAccount(request, "Ren");
  await rpc(request, `${FOLLOW}/UpdateFollowSettings`, { settings: { locked: true } }, me);
  await rpc(request, `${FOLLOW}/Follow`, { userId: me.userId }, ren);

  await page.goto("/friends/follow-requests");
  const row = page.getByTestId("follow-request").filter({ hasText: ren.displayName });
  await row.getByRole("button", { name: "削除", exact: true }).click();
  await expect(page.getByText("削除したよ", { exact: true })).toBeVisible();
  await expect(row).toBeHidden();
  await page.reload();
  await expect(page.getByText("届いているフォローリクエストはないよ")).toBeVisible();

  const incoming = await rpc<RequestsResponse>(
    request,
    `${FOLLOW}/ListFollowRequests`,
    { direction: "FOLLOW_REQUEST_DIRECTION_INCOMING" },
    me,
  );
  expect(incoming.requests ?? []).toHaveLength(0);
  const asRen = await rpc<SummaryResponse>(
    request,
    `${FOLLOW}/GetFollowSummary`,
    { userId: me.userId },
    ren,
  );
  expect(asRen.summary.relation?.following).toBe("FOLLOW_STATE_REQUESTED");

  // Ren's own screens still show the request as waiting.
  const context = await browser.newContext({
    baseURL: testInfo.project.use.baseURL,
    locale: "ja-JP",
    timezoneId: "Asia/Tokyo",
    viewport: testInfo.project.use.viewport,
    hasTouch: testInfo.project.use.hasTouch,
  });
  try {
    const renPage = await context.newPage();
    await signInAs(renPage, ren);
    await renPage.goto(`/u/${me.handle}`);
    await expect(renPage.getByRole("button", { name: "リクエスト済み" })).toBeVisible();
    await renPage.goto("/friends/follow-requests");
    await expect(
      renPage.getByTestId("follow-request-sent").filter({ hasText: me.displayName }),
    ).toBeVisible();
  } finally {
    await context.close();
  }
});

test("follow suggestions on さがす: follow back someone who follows you, dismiss another", async ({
  page,
  request,
  me,
}) => {
  const kai = await createAccount(request, "Kai");
  const nao = await createAccount(request, "Nao");
  await rpc(request, `${FOLLOW}/Follow`, { userId: me.userId }, kai);
  await rpc(request, `${FOLLOW}/Follow`, { userId: me.userId }, nao);

  await page.goto("/friends");
  await page.getByRole("tab", { name: "さがす" }).click();
  const suggestions = page.getByTestId("follow-suggestions");
  await expect(suggestions.getByText("おすすめアカウント")).toBeVisible();
  const kaiRow = suggestions.getByTestId("follow-suggestion").filter({ hasText: kai.displayName });
  const naoRow = suggestions.getByTestId("follow-suggestion").filter({ hasText: nao.displayName });
  await expect(kaiRow).toContainText("あなたをフォロー中");
  await expect(naoRow).toContainText("あなたをフォロー中");

  await kaiRow.getByRole("button", { name: "フォローバック" }).click();
  await expect(page.getByText(`${kai.displayName}さんをフォローしたよ!`)).toBeVisible();
  const followed = await rpc<SummaryResponse>(
    request,
    `${FOLLOW}/GetFollowSummary`,
    { userId: kai.userId },
    me,
  );
  expect(followed.summary.relation?.following).toBe("FOLLOW_STATE_FOLLOWING");
  // The row stays where it was, now as フォロー中.
  await expect(kaiRow.getByRole("button", { name: /フォロー中/ })).toBeVisible();

  await naoRow.getByRole("button", { name: `${nao.displayName}さんをおすすめに出さない` }).click();
  await expect(naoRow).toBeHidden();
  await page.reload();
  await page.getByRole("tab", { name: "さがす" }).click();
  await expect(
    page.getByTestId("follow-suggestion").filter({ hasText: nao.displayName }),
  ).toHaveCount(0);

  const listed = await rpc<{ suggestions?: { user?: { id: string } }[] }>(
    request,
    `${FOLLOW}/ListFollowSuggestions`,
    {},
    me,
  );
  expect((listed.suggestions ?? []).map((s) => s.user?.id)).not.toContain(nao.userId);
});

test("withdraw a pending request from the profile button and from the sent list", async ({
  page,
  request,
  me,
}) => {
  const sora = await createAccount(request, "Sora");
  const mio = await createAccount(request, "Mio");
  for (const locked of [sora, mio])
    await rpc(request, `${FOLLOW}/UpdateFollowSettings`, { settings: { locked: true } }, locked);
  const incomingOf = async (owner: typeof sora) =>
    (
      await rpc<RequestsResponse>(
        request,
        `${FOLLOW}/ListFollowRequests`,
        { direction: "FOLLOW_REQUEST_DIRECTION_INCOMING" },
        owner,
      )
    ).requests ?? [];

  // From the profile: リクエスト済み → 取り消す.
  await page.goto(`/u/${sora.handle}`);
  await page.getByRole("button", { name: "フォローする" }).click();
  await page.getByRole("button", { name: "リクエスト済み" }).click();
  const confirm = page.getByRole("dialog");
  await expect(
    confirm.getByText(`${sora.displayName}さんへのフォローリクエストを取り消すよ。`),
  ).toBeVisible();
  await confirm.getByRole("button", { name: "取り消す" }).click();
  await expect(page.getByText("リクエストを取り消したよ")).toBeVisible();
  await expect(page.getByRole("button", { name: "フォローする" })).toBeVisible();
  expect(await incomingOf(sora)).toHaveLength(0);

  // From the requests you sent.
  await rpc(request, `${FOLLOW}/Follow`, { userId: mio.userId }, me);
  await page.goto("/friends/follow-requests");
  const sent = page.getByTestId("follow-request-sent").filter({ hasText: mio.displayName });
  await sent.getByRole("button", { name: "キャンセル" }).click();
  await expect(page.getByText("リクエストを取り消したよ")).toBeVisible();
  await expect(sent).toBeHidden();
  expect(await incomingOf(mio)).toHaveLength(0);

  await page.goto(`/u/${mio.handle}`);
  await expect(page.getByRole("button", { name: "フォローする" })).toBeVisible();
});
