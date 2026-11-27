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
