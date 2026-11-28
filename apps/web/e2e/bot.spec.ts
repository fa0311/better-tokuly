import type { Page } from "@playwright/test";
import { holdAiProvider, offerStub, type Stub, startStub, withdrawStub } from "./local-ai";
import {
  type Account,
  adminAccount,
  expect as baseExpect,
  befriend,
  createAccount,
  rpc,
  test,
} from "./support";

// Tokuly Bot: talked to in its own conversation and mentioned in a group. Its
// "AI" is a local stand-in on the operator's network (see `local-ai.ts`); the
// bot answers through the job worker, a moment after the message.

const expect = baseExpect.configure({ timeout: 30_000 });
test.describe.configure({ timeout: 150_000 });

const ASSIST = "tokuly.chat.v1.AssistService";
const CONVERSATIONS = "tokuly.chat.v1.ConversationService";

/** What the stand-in says as the bot; everything else it is not asked here. */
const BOT_ANSWER = "駅前のカフェ、めっちゃおすすめだよ☕✨";
const answers = (system: string) =>
  system.includes("Tokuly Bot") ? { reply: BOT_ANSWER } : { text: "?" };

const composer = (page: Page) => page.getByRole("textbox", { name: "メッセージを入力" });

async function send(page: Page, text: string): Promise<void> {
  await composer(page).fill(text);
  await page.getByRole("button", { name: "送信", exact: true }).click();
}

/** Holds the server's AI provider for one test, pointed at a stand-in. */
async function withBot(
  request: Parameters<typeof adminAccount>[0],
  run: (stub: Stub, admin: Account) => Promise<void>,
): Promise<void> {
  const release = await holdAiProvider();
  const admin = await adminAccount(request);
  const stub = await startStub(answers);
  try {
    await offerStub(request, admin, stub);
    await run(stub, admin);
  } finally {
    await withdrawStub(request, admin);
    await stub.close();
    await release();
  }
}

test("Tokuly Bot is found from a new chat, answers in its conversation, and can be switched off", async ({
  page,
  request,
  me,
}, testInfo) => {
  await withBot(request, async (stub) => {
    await page.goto("/chat");
    await page.getByRole("button", { name: "新しいチャット" }).click();
    await page.getByTestId("talk-to-bot").click();

    // Its conversation opens, the name carrying the bot badge.
    await expect(page.getByTestId("conversation-title")).toHaveText("Tokuly Bot");
    await expect(page.locator("header").getByTestId("bot-badge")).toBeVisible();
    await send(page, "渋谷でおすすめのカフェある？");
    await expect(page.getByTestId("message").filter({ hasText: BOT_ANSWER })).toBeVisible();
    // What the stand-in got: the question, without the asker's name or handle.
    const sent = JSON.stringify(stub.requests.at(-1)?.body ?? {});
    expect(sent).toContain("渋谷でおすすめのカフェある？");
    expect(sent).not.toContain(me.handle);
    await page.screenshot({ path: testInfo.outputPath("bot-direct.png") });

    // Switched off in the AI settings: the conversation says so and can switch it on again.
    await page.goto("/settings/chat-ai");
    const section = page.getByTestId("bot-settings");
    const toggle = section.getByRole("switch", { name: "Tokuly Bot と話す" });
    await expect(toggle).toBeChecked();
    await toggle.click();
    await expect(toggle).not.toBeChecked();
    await page.goBack();
    await expect(page.getByTestId("bot-off")).toBeVisible();
    await page.getByTestId("bot-off").getByRole("button", { name: "オンにする" }).click();
    await expect(page.getByTestId("bot-off")).toBeHidden();
  });
});

test("in a group, Tokuly Bot answers when mentioned", async ({ page, request, me }, testInfo) => {
  await withBot(request, async () => {
    const aoi = await createAccount(request, "aoi");
    await befriend(request, me, aoi);
    const { conversation } = await rpc<{ conversation: { id: string } }>(
      request,
      `${CONVERSATIONS}/CreateGroup`,
      { name: "放課後メンバー", memberIds: [aoi.userId] },
      me,
    );
    const { settings } = await rpc<{ settings: { bot: { id: string } } }>(
      request,
      `${ASSIST}/GetAssistSettings`,
      {},
      me,
    );

    // Invited from the member list, without being anybody's friend.
    await page.goto(`/chat/${conversation.id}`);
    await page.getByRole("button", { name: "放課後メンバーのトーク情報" }).click();
    await page.getByRole("button", { name: "Tokuly Bot を招待" }).click();
    await expect(
      page.getByTestId("member-row").filter({ hasText: "Tokuly Bot" }).getByTestId("bot-badge"),
    ).toBeVisible();
    const members = await rpc<{ members: { user: { id: string } }[] }>(
      request,
      `${CONVERSATIONS}/ListMembers`,
      { conversationId: conversation.id },
      me,
    );
    expect(members.members.map((member) => member.user.id)).toContain(settings.bot.id);

    await page.goto(`/chat/${conversation.id}`);
    await send(page, "@tokuly_bot 放課後どこ行く？");
    const answer = page.getByTestId("message").filter({ hasText: BOT_ANSWER });
    await expect(answer).toBeVisible();
    await expect(answer.getByTestId("bot-badge")).toBeVisible();
    await page.screenshot({ path: testInfo.outputPath("bot-group.png") });
  });
});
