import { befriend, createAccount, expect, rpc, signInAs, test } from "./support";

test.describe.configure({ timeout: 120_000 });

test("a received request names the friends both know, whatever the sender shows", async ({
  page,
  request,
}) => {
  const year = new Date().getFullYear();
  const teen = await createAccount(request, "Teen", { birthdate: `${year - 15}-04-02` });
  const mio = await createAccount(request, "Mio");
  const sora = await createAccount(request, "Sora");
  const sender = await createAccount(request, "Sender");
  // A minor takes requests from friends of friends only: the teen asks first.
  for (const friend of [mio, sora]) {
    await befriend(request, teen, friend);
    await befriend(request, sender, friend);
  }
  const { settings } = await rpc<{ settings: { privacy: Record<string, unknown> } }>(
    request,
    "tokuly.identity.v1.AccountService/GetSettings",
    {},
    sender,
  );
  await rpc(
    request,
    "tokuly.identity.v1.AccountService/UpdateSettings",
    { privacy: { ...settings.privacy, friendListVisibleTo: "AUDIENCE_NOBODY" } },
    sender,
  );
  await rpc(
    request,
    "tokuly.social.v1.FriendService/SendFriendRequest",
    { userId: teen.userId },
    sender,
  );

  await signInAs(page, teen);
  await page.goto("/friends");
  await page.getByRole("tab", { name: /リクエスト/ }).click();
  await expect(page.getByText(/(Mio、Sora|Sora、Mio)と共通のともだち/)).toBeVisible();
});

test("a friend who unfriends or blocks me leaves my open list, a withdrawn request my badge", async ({
  page,
  me,
  request,
}) => {
  const aoi = await createAccount(request, "Aoi");
  const ren = await createAccount(request, "Ren");
  await befriend(request, me, aoi);
  await befriend(request, me, ren);
  const sender = await createAccount(request, "Sender");
  const sent = await rpc<{ request: { id: string } }>(
    request,
    "tokuly.social.v1.FriendService/SendFriendRequest",
    { userId: me.userId },
    sender,
  );

  await page.goto("/friends");
  const list = page.getByRole("main");
  await expect(list.getByText("Aoi", { exact: true })).toBeVisible();
  await expect(list.getByText("Ren", { exact: true })).toBeVisible();
  const requests = page.getByRole("tab", { name: /リクエスト/ });
  await expect(requests).toHaveText(/1/);

  // Both happen on the other side, while the screen is open: the realtime connection says so.
  await rpc(request, "tokuly.social.v1.FriendService/RemoveFriend", { userId: me.userId }, aoi);
  await rpc(request, "tokuly.social.v1.FriendService/BlockUser", { userId: me.userId }, ren);
  await expect(list.getByText("Aoi", { exact: true })).toHaveCount(0);
  await expect(list.getByText("Ren", { exact: true })).toHaveCount(0);

  await rpc(
    request,
    "tokuly.social.v1.FriendService/CancelFriendRequest",
    { requestId: sent.request.id },
    sender,
  );
  await expect(requests).toHaveText("リクエスト");
});
