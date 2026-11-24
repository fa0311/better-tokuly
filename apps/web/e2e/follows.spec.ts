import { befriend, createAccount, expect, rpc, test } from "./support";

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
