import { crc32, deflateSync } from "node:zlib";
import type { APIRequestContext, Locator, Page } from "@playwright/test";
import { type Account, befriend, createAccount, expect, rpc, signInAs, test } from "./support";

const BOX = "tokuly.ask.v1.QuestionBoxService";
const PROMPTS = "tokuly.ask.v1.PromptService";
const POLLS = "tokuly.ask.v1.ComplimentService";
const WALLET = "tokuly.wallet.v1.WalletService";

// ───────────────────────────── Arranging state ─────────────────────────────

interface Question {
  id: string;
  text: string;
  anonymous?: boolean;
  asker?: { handle: string };
  answer?: { text?: string };
  sentState?: string;
}

/** Opens an account's box. By default anyone signed in may ask, so tests need no friendships. */
async function openBox(
  request: APIRequestContext,
  owner: Account,
  settings: Record<string, unknown> = {},
): Promise<void> {
  await rpc(
    request,
    `${BOX}/UpdateSettings`,
    { boxOpen: true, audience: "ASKER_AUDIENCE_SIGNED_IN", ...settings },
    owner,
  );
}

async function ask(
  request: APIRequestContext,
  from: Account,
  to: Account,
  text: string,
  signed = false,
): Promise<Question> {
  return (
    await rpc<{ question: Question }>(
      request,
      `${BOX}/AskQuestion`,
      { user: to.userId, text, signed },
      from,
    )
  ).question;
}

async function inboxOf(
  request: APIRequestContext,
  owner: Account,
  folder = "QUESTION_FOLDER_INBOX",
): Promise<Question[]> {
  return (
    (await rpc<{ questions?: Question[] }>(request, `${BOX}/ListInbox`, { folder }, owner))
      .questions ?? []
  );
}

/** Waits until the delivery worker put a question into its owner's inbox, and returns it. */
async function delivered(
  request: APIRequestContext,
  owner: Account,
  text: string,
  folder?: string,
): Promise<Question> {
  let found: Question | undefined;
  await expect(async () => {
    found = (await inboxOf(request, owner, folder)).find((question) => question.text === text);
    expect(found, `"${text}" in the inbox`).toBeTruthy();
  }).toPass({ timeout: 20_000 });
  return found as Question;
}

/** `count` fresh accounts who are all friends with each other and with `me`: enough of a crowd for anything anonymous among friends. */
async function circleOf(
  request: APIRequestContext,
  me: Account,
  count: number,
): Promise<Account[]> {
  const friends: Account[] = [];
  for (let index = 0; index < count; index++)
    friends.push(await createAccount(request, `tomo${index + 1}`));
  const everyone = [me, ...friends];
  for (let a = 0; a < everyone.length; a++) {
    for (let b = a + 1; b < everyone.length; b++)
      await befriend(request, everyone[a]!, everyone[b]!);
  }
  return friends;
}

/** Buys the 100-coin package with the development payment provider. */
async function grantCoins(request: APIRequestContext, to: Account): Promise<void> {
  const shop = await rpc<{ packages: { id: string; sku: string }[] }>(
    request,
    `${WALLET}/ListCoinPackages`,
    {},
    to,
  );
  const hundred = shop.packages.find((candidate) => candidate.sku === "coins_100");
  expect(hundred, "the seeded coins_100 package").toBeTruthy();
  await rpc(request, `${WALLET}/StartPurchase`, { packageId: hundred?.id }, to);
  await expect
    .poll(
      async () => (await movementsOf(request, to, "purchase")).map((entry) => entry.coinsDelta),
      { timeout: 20_000 },
    )
    .toEqual(["100"]);
}

async function coinsOf(request: APIRequestContext, account: Account): Promise<string> {
  return (
    (await rpc<{ balance: { coins?: string } }>(request, `${WALLET}/GetBalance`, {}, account))
      .balance.coins ?? "0"
  );
}

/**
 * The movements of an account's coins made for `reason`, newest first.
 *
 * What an action pays or costs is read here, not from the balance: other
 * things move coins too (levelling up in quests grants some, whenever the XP
 * happens to arrive), so the balance at a given moment says little about one action.
 */
async function movementsOf(
  request: APIRequestContext,
  account: Account,
  reason: string,
): Promise<{ reason: string; coinsDelta?: string }[]> {
  const ledger = await rpc<{ entries?: { reason: string; coinsDelta?: string }[] }>(
    request,
    `${WALLET}/ListLedgerEntries`,
    { page: { limit: 100 } },
    account,
  );
  return (ledger.entries ?? []).filter((entry) => entry.reason === reason);
}

interface Round {
  state: string;
  poll?: { id: string; choices: { id: string }[] };
}

/** Has `voter` pick `target` in their current poll (the four choices of someone with four friends are those four). */
async function sendFlame(
  request: APIRequestContext,
  voter: Account,
  target: Account,
): Promise<void> {
  const { round } = await rpc<{ round: Round }>(request, `${POLLS}/GetRound`, {}, voter);
  expect(
    round.poll?.choices.map((choice) => choice.id),
    "the voter's poll offers the target",
  ).toContain(target.userId);
  await rpc(request, `${POLLS}/Vote`, { pollId: round.poll?.id, userId: target.userId }, voter);
}

/** A valid PNG of one colour, so image uploads need no fixture file. */
function solidPng(
  width: number,
  height: number,
  [red, green, blue]: [number, number, number],
): Buffer {
  const chunk = (type: string, data: Buffer) => {
    const body = Buffer.concat([Buffer.from(type, "latin1"), data]);
    const length = Buffer.alloc(4);
    length.writeUInt32BE(data.length);
    const checksum = Buffer.alloc(4);
    checksum.writeUInt32BE(crc32(body));
    return Buffer.concat([length, body, checksum]);
  };
  const header = Buffer.alloc(13);
  header.writeUInt32BE(width, 0);
  header.writeUInt32BE(height, 4);
  header.set([8, 2, 0, 0, 0], 8); // 8-bit RGB, no interlacing
  const row = Buffer.concat([
    Buffer.from([0]),
    Buffer.alloc(width * 3, Buffer.from([red, green, blue])),
  ]);
  const pixels = Buffer.concat(Array.from({ length: height }, () => row));
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk("IHDR", header),
    chunk("IDAT", deflateSync(pixels)),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}

// ───────────────────────────── Things on screen ─────────────────────────────

/** Opens a page of the app and waits for the signed-in frame (the first visit after a change loads many modules). */
async function open(page: Page, url: string): Promise<void> {
  await page.goto(url);
  await expect(page.getByRole("main")).toBeVisible({ timeout: 45_000 });
}

/** Resolves once the page's realtime connection is up, so that nothing sent afterwards is missed. */
async function live(page: Page): Promise<void> {
  await expect(page.getByRole("navigation", { name: "main" }).first()).toBeVisible();
  const connected = () => {
    const connection = (window as { __tokulyRealtime?: { connectionStatus: string } })
      .__tokulyRealtime;
    return connection ? connection.connectionStatus === "connected" : null;
  };
  if ((await page.evaluate(connected)) === null) await page.waitForTimeout(3000);
  else await page.waitForFunction(connected, null, { timeout: 20_000 });
}

const questionRow = (page: Page, text: string): Locator =>
  page.getByTestId("question-row").filter({ hasText: text });
const qaCard = (page: Page, text: string): Locator =>
  page.getByTestId("qa-card").filter({ hasText: text });

/** An amount as the `Coins` glyph writes it: the coin's "T", then the number. */
const coins = (amount: number) => new RegExp(`^T\\+?${amount.toLocaleString("en-US")}$`);

// ───────────────────────────── My question box ─────────────────────────────

test("the hub opens the box, and the share sheet has the public link and an image card", async ({
  page,
  request,
  me,
}) => {
  await open(page, "/ask");
  const box = page.getByTestId("my-box");
  await expect(box.getByTestId("box-state")).toHaveText("おやすみ中");

  await box.getByRole("switch").click();
  await expect(box.getByTestId("box-state")).toHaveText("受付中");
  await expect
    .poll(
      async () =>
        (await rpc<{ settings: { boxOpen?: boolean } }>(request, `${BOX}/GetSettings`, {}, me))
          .settings.boxOpen,
    )
    .toBe(true);

  await box.getByRole("button", { name: "シェアする" }).click();
  const sheet = page.getByRole("dialog");
  await expect(sheet.getByTestId("share-url")).toHaveText(new RegExp(`/q/${me.handle}$`));
  // The card is drawn in the browser: a story-sized picture.
  const card = sheet.getByTestId("share-card");
  await expect(card).toBeVisible({ timeout: 20_000 });
  expect(
    await card.evaluate((image: HTMLImageElement) =>
      image.decode().then(() => [image.naturalWidth, image.naturalHeight]),
    ),
  ).toEqual([1080, 1920]);
});

test("the prompt line, who may ask and the word filter are settings of the box", async ({
  page,
  request,
  me,
}) => {
  const asker = await createAccount(request, "asker");
  await openBox(request, me);
  await open(page, "/ask/settings");

  await page.getByLabel("ひとこと").fill("恋バナ大歓迎!");
  await page.getByRole("button", { name: "保存する" }).click();
  await expect(page.getByText("ひとことを変えたよ!")).toBeVisible();

  await page.getByLabel("NGワード").fill("きらい");
  await page.getByRole("button", { name: "追加" }).click();
  await expect(page.getByTestId("muted-words")).toContainText("きらい");

  // A question with the muted word is held in the filter folder; the asker cannot tell.
  await ask(request, asker, me, "きらいな食べ物は?");
  await delivered(request, me, "きらいな食べ物は?", "QUESTION_FOLDER_FILTERED");
  await open(page, "/ask/inbox");
  await expect(page.getByText("まだ質問は届いてないよ")).toBeVisible();
  await page.getByRole("tab", { name: /フィルター/ }).click();
  await expect(questionRow(page, "きらいな食べ物は?")).toBeVisible();

  // The box page shows the new prompt line to the asker.
  const box = (
    await rpc<{ box: { prompt: string } }>(request, `${BOX}/GetBox`, { user: me.handle }, asker)
  ).box;
  expect(box.prompt).toBe("恋バナ大歓迎!");
});

// ───────────────────────────── Asking ─────────────────────────────

test("a question asked anonymously from a second account arrives live, without a name", async ({
  page,
  request,
  me,
}) => {
  const asker = await createAccount(request, "himitsu");
  await openBox(request, me);
  await open(page, "/ask/inbox");
  await expect(page.getByText("まだ質問は届いてないよ")).toBeVisible();
  await live(page);

  await ask(request, asker, me, "好きな人いる?");
  const row = questionRow(page, "好きな人いる?");
  await expect(row).toBeVisible({ timeout: 20_000 });
  await expect(row.getByTestId("sender")).toHaveText(/匿名さん/);
  await expect(row).not.toContainText(asker.displayName);
  await expect(page.getByRole("main")).not.toContainText(asker.handle);
});

test("asking on someone's box is anonymous by default and says what they will see", async ({
  page,
  request,
  me: _me,
}) => {
  const owner = await createAccount(request, "hakonushi");
  await openBox(request, owner, { boxPrompt: "なんでもきいて!" });
  await open(page, `/ask/u/${owner.handle}`);
  await expect(page.getByTestId("box-prompt")).toContainText("なんでもきいて!");

  await expect(page.getByRole("radio", { name: "匿名で送る" })).toBeChecked();
  await expect(page.getByTestId("privacy-note")).toContainText("だれが送ったかは相手に見えないよ");
  await page.getByPlaceholder("質問を書いてね").fill("朝ごはんはパン派?");
  await expect(page.getByText("9 / 300")).toBeVisible();
  await page.getByRole("button", { name: "質問を送る" }).click();
  await expect(page.getByTestId("ask-sent")).toContainText("質問を送ったよ!");

  const arrived = await delivered(request, owner, "朝ごはんはパン派?");
  expect(arrived.anonymous).toBe(true);
  expect(arrived.asker).toBeUndefined();

  // The asker's own list knows it as waiting.
  await page.getByRole("link", { name: "送った質問を見る" }).click();
  const sent = page.getByTestId("sent-row").filter({ hasText: "朝ごはんはパン派?" });
  await expect(sent.getByTestId("sent-state")).toContainText("回答まち");
});

test("a signed question shows the asker's name to the owner", async ({ page, request, me }) => {
  const owner = await createAccount(request, "hakonushi");
  await openBox(request, owner);
  await open(page, `/ask/u/${owner.handle}`);

  await page.getByRole("radio", { name: "名前つきで送る" }).click();
  await expect(page.getByTestId("privacy-note")).toContainText("あなたの名前が見えるよ");
  await page.getByPlaceholder("質問を書いてね").fill("今度あそぼ!");
  await page.getByRole("button", { name: "質問を送る" }).click();
  await expect(page.getByTestId("ask-sent")).toBeVisible();
  await delivered(request, owner, "今度あそぼ!");

  // The owner's inbox names the asker.
  await signInAs(page, owner);
  await open(page, "/ask/inbox");
  await expect(questionRow(page, "今度あそぼ!").getByTestId("sender")).toContainText(
    me.displayName,
  );
});

test("an anonymous question can carry a picture when the owner takes pictures", async ({
  page,
  request,
  me: _me,
}) => {
  const owner = await createAccount(request, "hakonushi");
  await openBox(request, owner, { allowImages: true });
  await open(page, `/ask/u/${owner.handle}`);

  await page.getByTestId("image-input").setInputFiles({
    name: "photo.png",
    mimeType: "image/png",
    buffer: solidPng(320, 240, [255, 92, 157]),
  });
  await expect(page.getByText("画像は相手に送られるよ")).toBeVisible();
  await page.getByPlaceholder("質問を書いてね").fill("これどこだと思う?");
  // The button waits for the upload to be processed.
  await expect(page.getByRole("button", { name: "質問を送る" })).toBeEnabled({ timeout: 30_000 });
  await page.getByRole("button", { name: "質問を送る" }).click();
  await expect(page.getByTestId("ask-sent")).toBeVisible();

  const arrived = (await delivered(request, owner, "これどこだと思う?")) as Question & {
    image?: { url?: string };
  };
  expect(arrived.anonymous).toBe(true);
  expect(arrived.image?.url).toBeTruthy();

  // The owner sees the picture on the question.
  await signInAs(page, owner);
  await open(page, "/ask/inbox");
  await expect(questionRow(page, "これどこだと思う?").locator("img")).toBeVisible();
});

test("a friends-only box whose owner has few friends takes signed questions only, and says why", async ({
  page,
  request,
  me,
}) => {
  const owner = await createAccount(request, "shinyu");
  await befriend(request, me, owner);
  await rpc(
    request,
    `${BOX}/UpdateSettings`,
    { boxOpen: true, audience: "ASKER_AUDIENCE_FRIENDS" },
    owner,
  );
  await open(page, `/ask/u/${owner.handle}`);

  await expect(page.getByTestId("privacy-note")).toContainText("名前つきでだけ送れるよ");
  await expect(page.getByRole("radio", { name: "匿名で送る" })).toHaveCount(0);
  await page.getByPlaceholder("質問を書いてね").fill("テスト勉強してる?");
  await page.getByRole("button", { name: "質問を送る" }).click();
  await expect(page.getByTestId("ask-sent")).toBeVisible();
  expect((await delivered(request, owner, "テスト勉強してる?")).asker?.handle).toBe(me.handle);
});

test("a closed box and a friends-only box turn strangers away kindly", async ({
  page,
  request,
  me: _me,
}) => {
  const closed = await createAccount(request, "oyasumi");
  const friendsOnly = await createAccount(request, "nakayoshi");
  await rpc(
    request,
    `${BOX}/UpdateSettings`,
    { boxOpen: true, audience: "ASKER_AUDIENCE_FRIENDS" },
    friendsOnly,
  );

  await open(page, `/ask/u/${closed.handle}`);
  await expect(page.getByTestId("cannot-ask")).toContainText("いまは質問をお休み中だよ");
  await open(page, `/ask/u/${friendsOnly.handle}`);
  await expect(page.getByTestId("cannot-ask")).toContainText("ともだちだけが質問できるよ");
  await expect(page.getByPlaceholder("質問を書いてね")).toHaveCount(0);
});

// ───────────────────────────── Inbox and answers ─────────────────────────────

test("a public answer appears on the Q&A page, on the profile, and on the public page signed out", async ({
  page,
  request,
  me,
  browser,
  baseURL,
}) => {
  const asker = await createAccount(request, "asker");
  await openBox(request, me);
  await ask(request, asker, me, "得意な教科は?");
  await delivered(request, me, "得意な教科は?");

  await open(page, "/ask/inbox");
  await questionRow(page, "得意な教科は?")
    .getByRole("button", { name: "答える", exact: true })
    .click();
  const composer = page.getByRole("dialog");
  await expect(composer.getByRole("radio", { name: "みんな" })).toBeChecked();
  await composer.getByPlaceholder("回答を書いてね").fill("体育だけは自信ある!");
  await composer.getByRole("button", { name: "回答する" }).click();
  await expect(page.getByText("回答したよ!")).toBeVisible();
  await expect(questionRow(page, "得意な教科は?")).toHaveCount(0);

  // The answered tab, the owner's Q&A page and their profile.
  await page.getByRole("tab", { name: "回答済み" }).click();
  await expect(qaCard(page, "得意な教科は?")).toContainText("体育だけは自信ある!");
  await open(page, `/ask/u/${me.handle}`);
  await expect(qaCard(page, "得意な教科は?")).toContainText("体育だけは自信ある!");
  await open(page, `/u/${me.handle}`);
  await expect(page.getByTestId("profile-answers")).toContainText("体育だけは自信ある!");

  // Someone without an account sees the box and the answer, and is led to sign up to ask.
  const guest = await browser.newContext({
    baseURL,
    locale: "ja-JP",
    viewport: page.viewportSize(),
  });
  try {
    const visitor = await guest.newPage();
    await visitor.goto(`/q/${me.handle}`);
    const card = visitor.getByTestId("qa-card").filter({ hasText: "得意な教科は?" });
    await expect(card).toContainText("体育だけは自信ある!", { timeout: 45_000 });
    await expect(card.getByTestId("sender")).toHaveText(/匿名さん/);
    await expect(visitor.getByRole("button", { name: "はじめる（無料）" })).toBeVisible();
    await visitor.getByPlaceholder("質問を書いてね").fill("ゲストの質問");
    await visitor.getByRole("button", { name: "ログインして質問する" }).click();
    await expect(visitor).toHaveURL(/\/login\?redirect=/);
  } finally {
    await guest.close();
  }

  // The asker is told: their question is answered, and opens to the answer.
  await signInAs(page, asker);
  await open(page, "/ask/sent");
  const sent = page.getByTestId("sent-row").filter({ hasText: "得意な教科は?" });
  await sent.getByRole("link", { name: /回答きたよ!/ }).click();
  await expect(page.getByTestId("sent-by-me")).toBeVisible();
  await expect(qaCard(page, "得意な教科は?")).toContainText("体育だけは自信ある!");
});

test("the asker sees their question turn answered live, and a private answer stays between the two", async ({
  page,
  request,
  me,
}) => {
  const owner = await createAccount(request, "hakonushi");
  const outsider = await createAccount(request, "soto");
  await openBox(request, owner);
  const question = await ask(request, me, owner, "ひみつの質問");
  await delivered(request, owner, "ひみつの質問");

  await open(page, "/ask/sent");
  const sent = page.getByTestId("sent-row").filter({ hasText: "ひみつの質問" });
  await expect(sent.getByTestId("sent-state")).toContainText("回答まち");
  await live(page);

  await rpc(
    request,
    `${BOX}/AnswerQuestion`,
    { questionId: question.id, text: "ここだけの話ね", audience: "ANSWER_AUDIENCE_PRIVATE" },
    owner,
  );
  await expect(sent.getByTestId("sent-state")).toContainText("回答きたよ!", { timeout: 20_000 });
  await sent.getByRole("link", { name: /回答きたよ!/ }).click();
  await expect(qaCard(page, "ひみつの質問")).toContainText("ここだけの話ね");
  await expect(page.getByText(/この回答はあなたと.+だけが見られるよ/)).toBeVisible();

  // Nobody else gets it, and it is not on the Q&A page.
  const seenByOutsider = await rpc<{ question: Question }>(
    request,
    `${BOX}/GetQuestion`,
    { questionId: question.id },
    outsider,
  ).catch(() => undefined);
  expect(seenByOutsider?.question.answer).toBeUndefined();
  expect(
    (
      await rpc<{ questions?: Question[] }>(
        request,
        `${BOX}/ListAnswers`,
        { user: owner.userId },
        outsider,
      )
    ).questions ?? [],
  ).toHaveLength(0);
});

test("an answer can be edited, limited to friends and taken back", async ({
  page,
  request,
  me,
}) => {
  const asker = await createAccount(request, "asker");
  await openBox(request, me);
  const question = await ask(request, asker, me, "将来の夢は?");
  await delivered(request, me, "将来の夢は?");
  await rpc(request, `${BOX}/AnswerQuestion`, { questionId: question.id, text: "まだひみつ" }, me);

  await open(page, "/ask/inbox?tab=answered");
  const card = qaCard(page, "将来の夢は?");
  await card.getByRole("button", { name: "その他" }).click();
  await page.getByRole("menuitem", { name: "回答を編集" }).click();
  const composer = page.getByRole("dialog");
  await expect(composer.getByPlaceholder("回答を書いてね")).toHaveValue("まだひみつ");
  await composer.getByPlaceholder("回答を書いてね").fill("ゲームをつくる人!");
  await composer.getByRole("radio", { name: "ともだち" }).click();
  await composer.getByRole("button", { name: "保存する" }).click();
  await expect(page.getByText("回答を変えたよ!")).toBeVisible();
  await expect(card).toContainText("ゲームをつくる人!");
  await expect(card).toContainText("ともだち限定");
  await expect(card).toContainText("編集済み");
  // No longer for everyone: the public Q&A page is empty again.
  await expect
    .poll(
      async () =>
        (
          (
            await rpc<{ questions?: Question[] }>(request, `${BOX}/ListAnswers`, {
              user: me.userId,
            })
          ).questions ?? []
        ).length,
    )
    .toBe(0);

  await card.getByRole("button", { name: "その他" }).click();
  await page.getByRole("menuitem", { name: "回答を取り消す" }).click();
  await page.getByRole("dialog").getByRole("button", { name: "取り消す" }).click();
  await expect(page.getByText("回答を取り消したよ")).toBeVisible();
  await expect(card).toHaveCount(0);
  await page.getByRole("tab", { name: /新着/ }).click();
  await expect(questionRow(page, "将来の夢は?")).toBeVisible();
});

test("blocking a sender keeps their next question out, without saying who it was", async ({
  page,
  request,
  me,
}) => {
  const troll = await createAccount(request, "arashi");
  const friend = await createAccount(request, "yasashii");
  await openBox(request, me);
  await ask(request, troll, me, "いやなこと");
  await delivered(request, me, "いやなこと");

  await open(page, "/ask/inbox");
  await questionRow(page, "いやなこと").getByRole("button", { name: "その他" }).click();
  await page.getByRole("menuitem", { name: "この送り主をブロック" }).click();
  const confirm = page.getByRole("dialog");
  await expect(confirm).toContainText("だれなのかは表示されないし、相手にも伝わらないよ");
  await confirm.getByRole("button", { name: "ブロックする" }).click();
  await expect(page.getByText("この送り主をブロックしたよ")).toBeVisible();
  await live(page);

  // The blocked sender's request succeeds like anyone's; only the kind one arrives.
  const dropped = await ask(request, troll, me, "またきたよ");
  expect(dropped.sentState).toBe("SENT_STATE_WAITING");
  await ask(request, friend, me, "げんき?");
  await expect(questionRow(page, "げんき?")).toBeVisible({ timeout: 20_000 });
  await expect(questionRow(page, "またきたよ")).toHaveCount(0);
  expect((await inboxOf(request, me)).map((question) => question.text)).not.toContain("またきたよ");

  // The block is listed by what it was made from, and can be lifted.
  await open(page, "/ask/settings");
  const blocks = page.getByTestId("sender-blocks");
  await expect(blocks).toContainText("いやなこと");
  await expect(blocks).not.toContainText(troll.displayName);
  await blocks.getByRole("button", { name: "解除" }).click();
  await expect(page.getByText("ブロックした送り主はいないよ")).toBeVisible();
});

test("a question its sender took back can still be blocked and reported", async ({
  page,
  request,
  me,
}) => {
  const troll = await createAccount(request, "arashi");
  await openBox(request, me);
  const question = await ask(request, troll, me, "やっぱりなし");
  await delivered(request, me, "やっぱりなし");
  await open(page, "/ask/inbox");
  const row = questionRow(page, "やっぱりなし");
  await expect(row).toBeVisible();
  // Let the page finish what it reads when its connection comes up: a list read after the withdrawal no longer has the question.
  await live(page);
  await page.waitForLoadState("networkidle");

  // The sender withdraws it while it is on the owner's screen.
  await rpc(request, `${BOX}/WithdrawQuestion`, { questionId: question.id }, troll);
  expect((await inboxOf(request, me)).map((item) => item.text)).not.toContain("やっぱりなし");

  await row.getByRole("button", { name: "その他" }).click();
  await page.getByRole("menuitem", { name: "通報する" }).click();
  const report = page.getByRole("dialog");
  await report.getByRole("radio", { name: /スパム/ }).click();
  await report.getByRole("button", { name: "通報する" }).click();
  await expect(page.getByText("通報を受け付けたよ")).toBeVisible();

  await row.getByRole("button", { name: "その他" }).click();
  await page.getByRole("menuitem", { name: "この送り主をブロック" }).click();
  await page.getByRole("dialog").getByRole("button", { name: "ブロックする" }).click();
  await expect(page.getByText("この送り主をブロックしたよ")).toBeVisible();
  const blocks =
    (await rpc<{ blocks?: { preview: string }[] }>(request, `${BOX}/ListSenderBlocks`, {}, me))
      .blocks ?? [];
  expect(blocks.map((block) => block.preview)).toEqual(["やっぱりなし"]);

  // The block holds: what they send next does not arrive.
  await ask(request, troll, me, "もういっかい");
  await ask(request, await createAccount(request, "yasashii"), me, "こんにちは!");
  await delivered(request, me, "こんにちは!");
  expect((await inboxOf(request, me)).map((item) => item.text)).not.toContain("もういっかい");
});

test("a question can be archived, brought back and deleted from its menu", async ({
  page,
  request,
  me,
}) => {
  const asker = await createAccount(request, "asker");
  await openBox(request, me);
  await ask(request, asker, me, "あとで答えてね");
  await delivered(request, me, "あとで答えてね");
  await open(page, "/ask/inbox");

  await questionRow(page, "あとで答えてね").getByRole("button", { name: "その他" }).click();
  await page.getByRole("menuitem", { name: "アーカイブ" }).click();
  await expect(questionRow(page, "あとで答えてね")).toHaveCount(0);
  await page.getByRole("tab", { name: "アーカイブ" }).click();
  await expect(questionRow(page, "あとで答えてね")).toBeVisible();

  await questionRow(page, "あとで答えてね").getByRole("button", { name: "その他" }).click();
  await page.getByRole("menuitem", { name: "受信箱にもどす" }).click();
  await page.getByRole("tab", { name: /新着/ }).click();
  await expect(questionRow(page, "あとで答えてね")).toBeVisible();

  await questionRow(page, "あとで答えてね").getByRole("button", { name: "その他" }).click();
  await page.getByRole("menuitem", { name: "削除する" }).click();
  await page.getByRole("dialog").getByRole("button", { name: "削除する" }).click();
  await expect(questionRow(page, "あとで答えてね")).toHaveCount(0);
  await expect.poll(async () => (await inboxOf(request, me)).length).toBe(0);
});

test("on a phone, swiping a question left archives it", async ({ page, request, me }, testInfo) => {
  test.skip(testInfo.project.name !== "phone", "swipe actions are a touch gesture");
  const asker = await createAccount(request, "asker");
  await openBox(request, me);
  await ask(request, asker, me, "スワイプしてね");
  await delivered(request, me, "スワイプしてね");
  await open(page, "/ask/inbox");

  const row = page.getByTestId("swipe-row").filter({ hasText: "スワイプしてね" });
  await expect(row).toBeVisible();
  const box = (await row.boundingBox())!;
  const y = box.y + box.height / 2;
  const pointer = (type: string, x: number) =>
    row.dispatchEvent(type, {
      pointerId: 7,
      pointerType: "touch",
      isPrimary: true,
      clientX: x,
      clientY: y,
      bubbles: true,
    });
  await pointer("pointerdown", box.x + box.width - 30);
  for (const dx of [20, 60, 110, 150]) await pointer("pointermove", box.x + box.width - 30 - dx);
  await pointer("pointerup", box.x + box.width - 180);

  await expect(page.getByText("アーカイブしたよ")).toBeVisible();
  await expect(questionRow(page, "スワイプしてね")).toHaveCount(0);
  expect((await delivered(request, me, "スワイプしてね", "QUESTION_FOLDER_ARCHIVED")).text).toBe(
    "スワイプしてね",
  );
});

test("an answered question takes likes and comments on its page", async ({ page, request, me }) => {
  const owner = await createAccount(request, "hakonushi");
  await openBox(request, owner);
  const question = await ask(request, me, owner, "おすすめの曲は?");
  await delivered(request, owner, "おすすめの曲は?");
  await rpc(
    request,
    `${BOX}/AnswerQuestion`,
    { questionId: question.id, text: "さいきんはボカロ!" },
    owner,
  );

  await open(page, `/ask/q/${question.id}`);
  const card = qaCard(page, "おすすめの曲は?");
  await expect(card).toContainText("さいきんはボカロ!");
  await card.getByRole("button", { name: /いいね/ }).click();
  await expect
    .poll(
      async () =>
        (
          await rpc<{ question: { likeCount?: number } }>(
            request,
            `${BOX}/GetQuestion`,
            { questionId: question.id },
            owner,
          )
        ).question.likeCount ?? 0,
      { timeout: 20_000 },
    )
    .toBe(1);
  await page.getByPlaceholder("コメントを書く…").fill("わかる、ボカロいいよね");
  await page.getByPlaceholder("コメントを書く…").press("Enter");
  await expect(page.getByText("わかる、ボカロいいよね")).toBeVisible();
});

// ───────────────────────────── Prompts ─────────────────────────────

test("a prompt round trip: post it, a friend's response arrives live, publish it for friends", async ({
  page,
  request,
  me,
}) => {
  const [friend, other] = await circleOf(request, me, 3);
  await open(page, "/ask/prompts");
  await page.getByRole("button", { name: "お題をつくる" }).first().click();
  const form = page.getByRole("dialog");
  await form.getByRole("button", { name: "私の第一印象は?" }).click();
  await form.getByRole("button", { name: "ともだちに出す" }).click();

  await expect(page).toHaveURL(/\/ask\/prompts\/[0-9a-f-]+$/);
  const promptId = page.url().split("/").pop() as string;
  await expect(page.getByTestId("prompt-header")).toContainText("私の第一印象は?");
  await expect(page.getByText("まだ回答は届いてないよ")).toBeVisible();
  await live(page);

  // The friend finds it among their friends' prompts and responds anonymously.
  const listed = await rpc<{ prompts?: { id: string }[] }>(
    request,
    `${PROMPTS}/ListFriendPrompts`,
    {},
    friend!,
  );
  expect((listed.prompts ?? []).map((prompt) => prompt.id)).toContain(promptId);
  await rpc(
    request,
    `${PROMPTS}/RespondToPrompt`,
    { promptId, text: "しっかり者っぽい!" },
    friend!,
  );

  const response = page.getByTestId("response-row").filter({ hasText: "しっかり者っぽい!" });
  await expect(response).toBeVisible({ timeout: 20_000 });
  await expect(response.getByTestId("sender")).toHaveText(/匿名さん/);
  await response.getByRole("button", { name: "公開する", exact: true }).click();
  await expect(response).toContainText("公開中");

  // Another friend reads it on the prompt's page.
  await expect
    .poll(async () =>
      (
        (
          await rpc<{ responses?: { text: string }[] }>(
            request,
            `${PROMPTS}/ListPublishedResponses`,
            { promptId },
            other!,
          )
        ).responses ?? []
      ).map((item) => item.text),
    )
    .toEqual(["しっかり者っぽい!"]);
});

test("responding to a friend's prompt from the list of friends' prompts", async ({
  page,
  request,
  me,
}) => {
  const [owner] = await circleOf(request, me, 3);
  const { prompt } = await rpc<{ prompt: { id: string } }>(
    request,
    `${PROMPTS}/CreatePrompt`,
    { text: "私を動物にたとえると?", emoji: "🐾" },
    owner!,
  );

  await open(page, "/ask/prompts");
  await page
    .getByTestId("friend-prompts")
    .getByRole("link", { name: /私を動物にたとえると\?/ })
    .click();
  await expect(page.getByTestId("prompt-header")).toContainText("私を動物にたとえると?");
  await page.getByPlaceholder("回答を書いてね").fill("ぜったい柴犬");
  await page.getByRole("button", { name: "回答を送る" }).click();
  await expect(page.getByTestId("my-response")).toContainText("ぜったい柴犬");

  await expect
    .poll(
      async () =>
        (
          (
            await rpc<{ responses?: { text: string; anonymous?: boolean }[] }>(
              request,
              `${PROMPTS}/ListPromptResponses`,
              { promptId: prompt.id },
              owner!,
            )
          ).responses ?? []
        ).map((item) => [item.text, item.anonymous]),
      {
        timeout: 20_000,
      },
    )
    .toEqual([["ぜったい柴犬", true]]);
});

// ───────────────────────────── Compliment polls ─────────────────────────────

test("a compliment round: pick a friend, they get a flame, and finishing pays the reward", async ({
  page,
  request,
  me,
}) => {
  test.setTimeout(120_000);
  // Five friends: four to show, one more for the shuffle to bring in.
  const friends = await circleOf(request, me, 5);
  await open(page, "/ask/polls");
  await expect(page.getByTestId("poll-progress")).toHaveText("1 / 12");
  await expect(page.getByTestId("poll-choices").getByRole("button")).toHaveCount(4);

  // Shuffle spends one of the three, skip moves on without a flame.
  await page.getByRole("button", { name: /シャッフル/ }).click();
  await expect(page.getByRole("button", { name: /シャッフル/ })).toContainText("あと2回");
  await page.getByRole("button", { name: "スキップ" }).click();
  await expect(page.getByTestId("poll-progress")).toHaveText("2 / 12");

  // Picking a friend sends them a flame; who picked is not part of it.
  const choice = page.getByTestId("poll-choices").getByRole("button").first();
  const pickedName = ((await choice.getAttribute("aria-label")) ?? "").replace("をえらぶ", "");
  const picked = friends.find((friend) => friend.displayName === pickedName) as Account;
  expect(picked, `the friend called ${pickedName}`).toBeTruthy();
  await choice.click();
  await expect(page.getByTestId("poll-progress")).toHaveText("3 / 12", { timeout: 15_000 });
  const flamesOf = async (account: Account) =>
    (await rpc<{ flames?: Record<string, unknown>[] }>(request, `${POLLS}/ListFlames`, {}, account))
      .flames ?? [];
  await expect.poll(async () => (await flamesOf(picked)).length, { timeout: 20_000 }).toBe(1);
  expect(JSON.stringify(await flamesOf(picked))).not.toContain(me.userId);

  // Play to the end: the reward (11 of 12 polls voted on) and the clock until the next round.
  for (let position = 3; position <= 12; position++) {
    await expect(page.getByTestId("poll-progress")).toHaveText(`${position} / 12`, {
      timeout: 15_000,
    });
    await page.getByTestId("poll-choices").getByRole("button").first().click();
  }
  const cooldown = page.getByTestId("poll-cooldown");
  await expect(cooldown).toBeVisible({ timeout: 15_000 });
  await expect(cooldown.getByTestId("poll-reward").getByText(coins(11))).toBeVisible();
  await expect(cooldown.getByTestId("poll-countdown")).toHaveText(/^\d{2}:\d{2}$/);
  // The round paid its reward, once.
  await expect
    .poll(
      async () => (await movementsOf(request, me, "ask.round")).map((entry) => entry.coinsDelta),
      { timeout: 20_000 },
    )
    .toEqual(["11"]);
});

test("someone without enough friends is sent to find some instead of a poll", async ({
  page,
  me: _me,
}) => {
  await open(page, "/ask/polls");
  await expect(page.getByText("ともだちがもうすこし必要みたい")).toBeVisible();
  await expect(
    page.getByRole("main").getByRole("link", { name: "ともだちをさがす" }),
  ).toHaveAttribute("href", "/friends");
});

test("buying a hint about a flame is confirmed and lowers the coin balance", async ({
  page,
  request,
  me,
}) => {
  const [voter] = await circleOf(request, me, 4);
  await sendFlame(request, voter!, me);
  await grantCoins(request, me);

  await open(page, "/ask/flames");
  const flame = page.getByTestId("flame-row").first();
  await expect(flame).toBeVisible({ timeout: 20_000 });
  await expect(flame).toContainText("ともだちのだれかがあなたをえらんだよ");
  await expect(page.getByTestId("flame-total")).toHaveText("🔥1");
  await expect(page.getByRole("main")).not.toContainText(voter!.displayName);

  // The dialog shows the price, what there is, and what will be left.
  const before = Number(await coinsOf(request, me));
  expect(before).toBeGreaterThanOrEqual(100);
  await flame.getByRole("button", { name: /ヒントを見る/ }).click();
  const dialog = page.getByRole("dialog");
  await expect(dialog.getByTestId("hint-price").getByText(coins(30))).toBeVisible();
  await expect(dialog.getByTestId("hint-balance").getByText(coins(before))).toBeVisible();
  await expect(dialog.getByTestId("hint-after").getByText(coins(before - 30))).toBeVisible();
  await dialog.getByRole("button", { name: "コインをつかう" }).click();

  // The toast says what the server charged, and the balance drops by exactly that.
  await expect(page.getByText("ヒントをゲット! 30コインつかったよ")).toBeVisible();
  await expect(flame.getByTestId("flame-hints").getByRole("listitem")).toHaveCount(1);
  expect((await movementsOf(request, me, "ask.hint")).map((entry) => entry.coinsDelta)).toEqual([
    "-30",
  ]);
  const after = Number(await coinsOf(request, me));
  // Nothing but the hint took coins away (a level-up in between could only add some).
  expect(after).toBeGreaterThanOrEqual(before - 30);
  await expect(page.getByRole("link", { name: new RegExp(String(after)) })).toBeVisible();
});

test("someone short on coins is sent to the wallet instead of being charged", async ({
  page,
  request,
  me,
}) => {
  const [voter] = await circleOf(request, me, 4);
  await sendFlame(request, voter!, me);

  await open(page, "/ask/flames");
  await page
    .getByTestId("flame-row")
    .first()
    .getByRole("button", { name: /ヒントを見る/ })
    .click({ timeout: 20_000 });
  const dialog = page.getByRole("dialog");
  await expect(dialog.getByTestId("hint-short")).toHaveText("コインが30たりないよ");
  await expect(dialog.getByRole("button", { name: "コインをつかう" })).toHaveCount(0);
  await dialog.getByRole("link", { name: "コインを買いにいく" }).click();
  await expect(page).toHaveURL(/\/wallet$/);
  expect(await coinsOf(request, me)).toBe("0");
});

// ───────────────────────────── Ways in ─────────────────────────────

test("the hub tile, a friend's profile and a visitor's draft all lead to a box", async ({
  page,
  request,
  me,
  browser,
  baseURL,
}) => {
  const owner = await createAccount(request, "hakonushi");
  await openBox(request, owner);

  await open(page, "/me");
  await expect(
    page.getByRole("main").getByRole("link", { name: "質問箱・ほめ投票" }),
  ).toHaveAttribute("href", "/ask");
  await open(page, `/u/${owner.handle}`);
  await page.getByRole("link", { name: "質問する" }).click();
  await expect(page).toHaveURL(new RegExp(`/ask/u/${owner.handle}$`));

  // A visitor writes a question first, then signs in: they land on the box with their words still there.
  const guest = await browser.newContext({
    baseURL,
    locale: "ja-JP",
    viewport: page.viewportSize(),
  });
  try {
    const visitor = await guest.newPage();
    await visitor.goto(`/q/${owner.handle}`);
    await visitor.getByPlaceholder("質問を書いてね").fill("はじめまして!");
    await visitor.getByRole("button", { name: "はじめる（無料）" }).click();
    await expect(visitor).toHaveURL(/\/signup/);
    await signInAs(visitor, me);
    await visitor.goto("/me");
    await expect(visitor).toHaveURL(new RegExp(`/ask/u/${owner.handle}$`), { timeout: 45_000 });
    await expect(visitor.getByPlaceholder("質問を書いてね")).toHaveValue("はじめまして!");
  } finally {
    await guest.close();
  }
});

test("what a visitor wrote stays in their tab: the next person to sign in on the device does not get it", async ({
  page,
  request,
  me,
  browser,
  baseURL,
}) => {
  const owner = await createAccount(request, "hakonushi");
  await openBox(request, owner);

  // A writes a question as a visitor, heads for sign-up, and leaves.
  const first = await browser.newContext({
    baseURL,
    locale: "ja-JP",
    viewport: page.viewportSize(),
  });
  let shared: Awaited<ReturnType<typeof first.storageState>>;
  try {
    const visitor = await first.newPage();
    await visitor.goto(`/q/${owner.handle}`);
    await visitor.getByPlaceholder("質問を書いてね").fill("だれにも言わないでね");
    await visitor.getByRole("button", { name: "はじめる（無料）" }).click();
    await expect(visitor).toHaveURL(/\/signup/);
    // Nothing of it is in the storage every tab of this browser shares.
    expect(await visitor.evaluate(() => JSON.stringify(window.localStorage))).not.toContain(
      "だれにも言わないでね",
    );
    shared = await first.storageState();
  } finally {
    await first.close();
  }

  // B signs in on the same browser (same cookies and local storage, another tab).
  const second = await browser.newContext({
    baseURL,
    locale: "ja-JP",
    viewport: page.viewportSize(),
    storageState: shared,
  });
  try {
    const other = await second.newPage();
    await signInAs(other, me);
    await other.goto("/me");
    await expect(other.getByRole("heading", { name: "じぶん" })).toBeVisible({ timeout: 45_000 });
    await expect(other).toHaveURL(/\/me$/);
    await other.goto(`/ask/u/${owner.handle}`);
    await expect(other.getByPlaceholder("質問を書いてね")).toHaveValue("");
    await expect(other.getByRole("main")).not.toContainText("だれにも言わないでね");
  } finally {
    await second.close();
  }
});

test("a box that does not exist says so, signed in and signed out", async ({
  page,
  me: _me,
  browser,
  baseURL,
}) => {
  await open(page, "/ask/u/nobody_here_0000");
  await expect(page.getByText("質問箱が見つかりません")).toBeVisible();

  const guest = await browser.newContext({
    baseURL,
    locale: "ja-JP",
    viewport: page.viewportSize(),
  });
  try {
    const visitor = await guest.newPage();
    await visitor.goto("/q/nobody_here_0000");
    await expect(visitor.getByText("質問箱が見つかりません")).toBeVisible({ timeout: 45_000 });
  } finally {
    await guest.close();
  }
});
