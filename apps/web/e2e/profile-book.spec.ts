import { expect as baseExpect, befriend, createAccount, rpc, signInAs, test } from "./support";

// Profile pages pull in every feature's sections; on a busy machine that takes a moment.
test.describe.configure({ timeout: 90_000 });
const expect = baseExpect.configure({ timeout: 20_000 });

const ACCOUNT = "tokuly.identity.v1.AccountService";

test("the プロフ帳 is filled in and each part reaches its own audience", async ({
  page,
  me,
  browser,
  request,
}) => {
  await page.goto("/settings/profile-book");
  await expect(page.getByRole("heading", { level: 1, name: "プロフ帳" })).toBeVisible();

  // About me: MBTI for friends only.
  await page.getByLabel("MBTI", { exact: true }).fill("enfp");
  const audiences = page.getByRole("combobox", { name: "見せる相手" });
  await audiences.first().click();
  await page.getByRole("option", { name: "ともだち", exact: true }).click();
  await page.getByRole("button", { name: "保存する" }).first().click();
  await expect(
    page.locator("[data-sonner-toast]").filter({ hasText: "プロフ帳を保存しました" }).first(),
  ).toBeVisible();

  // A question card from the bank, answered and saved.
  await page.getByRole("combobox", { name: "質問をえらぶ" }).click();
  await page.getByRole("option", { name: /無人島に1つ持っていくなら/ }).click();
  await page.getByRole("button", { name: "カードを追加" }).click();
  await page.getByTestId("card-editor").getByLabel("答え").fill("スマホの充電器");
  await page.getByRole("button", { name: "保存する" }).nth(1).click();
  await expect(page.getByTestId("card-editor")).toHaveCount(1);

  // A favourite thing, through the tag form.
  await page.getByLabel("なにが好き？").fill("ヨルシカ");
  await page.getByRole("button", { name: "追加", exact: true }).click();
  await expect(page.getByText("ヨルシカ").first()).toBeVisible();

  // My own profile shows it all.
  await page.goto(`/u/${me.handle}`);
  const book = page.getByRole("region", { name: "プロフ帳" });
  await expect(book.getByTestId("qa-card")).toContainText("スマホの充電器");
  await expect(book).toContainText("ENFP");

  // A stranger sees the public card, not the friends-only MBTI; a friend sees both.
  const stranger = await createAccount(request, "stranger");
  const friend = await createAccount(request, "friend");
  await befriend(request, me, friend);
  for (const [viewer, seesMbti] of [
    [stranger, false],
    [friend, true],
  ] as const) {
    const context = await browser.newContext();
    const other = await context.newPage();
    await signInAs(other, viewer);
    await other.goto(`/u/${me.handle}`);
    const theirs = other.getByRole("region", { name: "プロフ帳" });
    await expect(theirs.getByTestId("qa-card")).toContainText("スマホの充電器");
    if (seesMbti) await expect(theirs).toContainText("ENFP");
    else await expect(theirs).not.toContainText("ENFP");
    await context.close();
  }
});

test("hidden cards and only-me answers stay off other people's view", async ({
  page,
  me,
  request,
}) => {
  await rpc(
    request,
    `${ACCOUNT}/UpdateProfileExtras`,
    {
      qaCards: {
        items: [
          { promptId: "fav_food", answer: "カレー", audience: { visibility: "VISIBILITY_PUBLIC" } },
          {
            promptId: "dream",
            answer: "ひみつの夢",
            audience: { visibility: "VISIBILITY_PRIVATE" },
          },
          {
            promptId: "club",
            answer: "軽音部",
            audience: { visibility: "VISIBILITY_PUBLIC" },
            hidden: true,
          },
        ],
      },
    },
    me,
  );
  const viewer = await createAccount(request, "viewer");
  await signInAs(page, viewer);
  await page.goto(`/u/${me.handle}`);
  const cards = page.getByRole("region", { name: "プロフ帳" }).getByTestId("qa-card");
  await expect(cards).toHaveCount(1);
  await expect(cards).toContainText("カレー");
});

test("a birthday shared with friends shows on a friend's view of the profile", async ({
  page,
  me,
  browser,
  request,
}) => {
  await page.goto("/settings/privacy");
  await page.getByRole("combobox", { name: "誕生日を見せる相手" }).click();
  await page.getByRole("option", { name: /ともだちだけ/ }).click();
  await expect(page.getByRole("combobox", { name: "誕生日を見せる相手" })).toContainText(
    "ともだち",
  );
  // Saved on the server (the screen saves on change).
  await expect
    .poll(async () => {
      const settings = await rpc<{ settings: { privacy: { birthdayVisibleTo?: string } } }>(
        request,
        `${ACCOUNT}/GetSettings`,
        {},
        me,
      );
      return settings.settings.privacy.birthdayVisibleTo;
    })
    .toBe("AUDIENCE_FRIENDS");
  const friend = await createAccount(request, "pal");
  await befriend(request, me, friend);
  const context = await browser.newContext();
  const other = await context.newPage();
  await signInAs(other, friend);
  await other.goto(`/u/${me.handle}`);
  await expect(other.getByRole("region", { name: "プロフ帳" })).toContainText("4月2日");
  await context.close();
});
