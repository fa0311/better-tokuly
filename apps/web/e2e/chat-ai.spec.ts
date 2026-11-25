import http from "node:http";
import type { AddressInfo } from "node:net";
import type { APIRequestContext, Page } from "@playwright/test";
import {
  type Account,
  adminAccount,
  expect as baseExpect,
  befriend,
  createAccount,
  rpc,
  signInAs,
  test,
} from "./support";

// The AI assistant's providers: the operator's settings in the staff console,
// a person's choice in the chat settings, and the consent toggle in the
// privacy settings. The "AI" is a local stand-in speaking OpenAI's protocol,
// like LM Studio on the operator's network, reached through the operator's
// private-host allowance.

const expect = baseExpect.configure({ timeout: 20_000 });
test.describe.configure({ timeout: 120_000 });

const ASSIST = "tokuly.chat.v1.AssistService";
const ASSIST_ADMIN = "tokuly.chat.v1.AssistAdminService";

interface Stub {
  baseUrl: string;
  requests: { body: Record<string, unknown> }[];
  close(): Promise<void>;
}

/** A local OpenAI-compatible server answering by the system prompt, without schema support. */
async function startStub(): Promise<Stub> {
  const requests: Stub["requests"] = [];
  const server = http.createServer((incoming, outgoing) => {
    let raw = "";
    incoming.on("data", (chunk) => {
      raw += chunk;
    });
    incoming.on("end", () => {
      const body = JSON.parse(raw || "{}") as Record<string, unknown>;
      requests.push({ body });
      outgoing.setHeader("content-type", "application/json");
      if (body.response_format) {
        outgoing.statusCode = 400;
        outgoing.end(JSON.stringify({ error: "response_format is not supported" }));
        return;
      }
      const messages = body.messages as { content: string }[];
      const system = messages[0]?.content ?? "";
      const answer = system.includes("rewrite")
        ? { text: "ローカルAIで書きかえたよ！" }
        : system.includes("quick replies")
          ? { replies: ["いいね、行こう！", "何時にする？"] }
          : { translation: "Good morning!", source_language: "ja" };
      outgoing.end(
        JSON.stringify({
          choices: [
            {
              index: 0,
              message: {
                role: "assistant",
                content: `\`\`\`json\n${JSON.stringify(answer)}\n\`\`\``,
              },
              finish_reason: "stop",
            },
          ],
        }),
      );
    });
  });
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  const { port } = server.address() as AddressInfo;
  return {
    baseUrl: `http://127.0.0.1:${port}/v1`,
    requests,
    close: () => new Promise((resolve) => server.close(() => resolve())),
  };
}

async function directWith(request: APIRequestContext, a: Account, b: Account): Promise<string> {
  const { conversation } = await rpc<{ conversation: { id: string } }>(
    request,
    "tokuly.chat.v1.ConversationService/OpenDirect",
    { userId: b.userId },
    a,
  );
  return conversation.id;
}

const composer = (page: Page) => page.getByRole("textbox", { name: "メッセージを入力" });

test("an operator offers a local model, a person picks it, and other people's messages need their consent", async ({
  page,
  request,
  me,
}, testInfo) => {
  // The operator settings are one per server: one browser size changes them.
  test.skip(testInfo.project.name !== "desktop", "the operator settings are global");
  const stub = await startStub();
  const admin = await adminAccount(request);
  const adminPage = await page.context().browser()?.newPage({
    baseURL: testInfo.project.use.baseURL,
    locale: "ja-JP",
  });
  if (!adminPage) throw new Error("no browser");
  try {
    await signInAs(adminPage, admin);
    await adminPage.goto("/admin/chat-ai");
    const form = adminPage.getByRole("form", { name: "OpenAI互換（LM Studio など）" });
    await expect(form).toBeVisible();
    const enabled = form.getByRole("switch", { name: "有効にする" });
    if (!(await enabled.isChecked())) await enabled.click();
    await form.getByLabel("モデル").fill("local-qwen");
    await form.getByLabel("ベースURL").fill(stub.baseUrl);
    const allow = form.getByRole("switch", { name: /プライベートネットワークで許可/ });
    if (await allow.isChecked()) await allow.click();
    // A loopback address on its own port is refused without the allowance.
    await form.getByRole("button", { name: "保存" }).click();
    await expect(
      adminPage.getByText("このアドレスには接続できません", { exact: false }),
    ).toBeVisible();
    await adminPage
      .getByRole("form", { name: "OpenAI互換（LM Studio など）" })
      .getByRole("switch", { name: /プライベートネットワークで許可/ })
      .click();
    await adminPage
      .getByRole("form", { name: "OpenAI互換（LM Studio など）" })
      .getByRole("button", { name: "保存" })
      .click();
    await expect(
      adminPage
        .getByRole("form", { name: "OpenAI互換（LM Studio など）" })
        .getByText("使える", { exact: true }),
    ).toBeVisible();

    // The person picks the local model; the page says who receives the text.
    const friend = await createAccount(request, "aoi");
    await befriend(request, me, friend);
    const conversationId = await directWith(request, friend, me);
    await rpc(
      request,
      "tokuly.chat.v1.MessageService/SendMessage",
      { conversationId, clientId: crypto.randomUUID(), text: "おはよう！明日あそぼ" },
      friend,
    );
    await page.goto("/settings/chat-ai");
    // The choice is saved first and shown once the server took it.
    const local = page.getByRole("radio", { name: "OpenAI互換（LM Studio など）" });
    await local.click();
    await expect(local).toBeChecked();
    await expect(page.getByTestId("ai-recipient")).toHaveText("いまの送り先: 127.0.0.1");

    // In the conversation: asked first, naming the recipient, then reply ideas from the local model.
    await page.goto(`/chat/${conversationId}`);
    await expect(page.locator('[data-testid="message-list"][data-ready]')).toBeVisible();
    await page.getByRole("button", { name: "AIアシスト" }).click();
    await page.getByRole("menuitem", { name: "返信のヒントをもらう" }).click();
    const consent = page.getByRole("dialog").filter({ hasText: "AIアシストを使う前に" });
    await expect(consent).toContainText("127.0.0.1");
    await consent.getByRole("button", { name: "わかった、使う" }).click();
    await page
      .getByTestId("suggested-replies")
      .getByRole("button", { name: "いいね、行こう！" })
      .click();
    await expect(composer(page)).toHaveValue("いいね、行こう！");
    // The server without schema support was asked again for plain JSON.
    expect(stub.requests.some((request) => request.body.response_format)).toBeTruthy();
    expect(stub.requests.some((request) => !request.body.response_format)).toBeTruthy();
    expect(stub.requests.every((request) => request.body.model === "local-qwen")).toBeTruthy();

    // The friend says no: their messages are left out of reply ideas and cannot be translated.
    await rpc(request, `${ASSIST}/SetAiMessageConsent`, { allow: false }, friend);
    await rpc(
      request,
      "tokuly.chat.v1.MessageService/SendMessage",
      { conversationId, clientId: crypto.randomUUID(), text: "やっぱりないしょ" },
      friend,
    );
    await expect(page.getByTestId("message-list").getByText("やっぱりないしょ")).toBeVisible();
    const before = stub.requests.length;
    await composer(page).fill("");
    await page.getByRole("button", { name: "AIアシスト" }).click();
    await page.getByRole("menuitem", { name: "返信のヒントをもらう" }).click();
    await expect(page.getByTestId("suggested-replies")).toContainText("OKしてない人のメッセージ");
    expect(stub.requests.length).toBe(before);
  } finally {
    // Leave the server as it was: no compatible provider.
    await rpc(
      request,
      `${ASSIST_ADMIN}/UpdateAssistProvider`,
      { provider: "AI_PROVIDER_OPENAI_COMPATIBLE", enabled: false, model: "", baseUrl: "" },
      admin,
    ).catch(() => undefined);
    await adminPage.close();
    await stub.close();
  }
});

test("the consent toggle lives in the privacy settings and starts on for adults", async ({
  page,
  request,
  me,
}) => {
  await page.goto("/settings/privacy");
  const toggle = page.getByRole("switch", { name: "AI に自分のメッセージを使ってもいい" });
  await expect(toggle).toBeChecked();
  await expect(page.getByText("いまは年齢にあわせた初期設定だよ", { exact: false })).toBeVisible();
  await toggle.click();
  await expect(toggle).not.toBeChecked();
  await expect(async () => {
    const { settings } = await rpc<{
      settings: { allowMessageUse?: boolean; allowMessageUseIsDefault?: boolean };
    }>(request, `${ASSIST}/GetAssistSettings`, {}, me);
    expect(settings.allowMessageUse ?? false).toBe(false);
    expect(settings.allowMessageUseIsDefault ?? false).toBe(false);
  }).toPass();

  // The AI settings page is one row away and points back here.
  await page.goto("/settings");
  await page.getByRole("link", { name: /AIアシスト/ }).click();
  await expect(page).toHaveURL(/\/settings\/chat-ai$/);
  await expect(page.getByRole("heading", { name: "AIアシストの設定" })).toBeVisible();
});
