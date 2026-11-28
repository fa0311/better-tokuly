import type { APIRequestContext, Page } from "@playwright/test";
import { PROTOCOL } from "@tokuly/miniapp/protocol";
import { type SampleServer, serveSample } from "../../miniapp-sample/serve";
import {
  type Account,
  adminAccount,
  befriend,
  createAccount,
  expect,
  RpcError,
  rpc,
  takePolicyViolations,
  test,
} from "./support";

const MINI_APPS = "tokuly.dev.v1.MiniAppService";
const MESSAGES = "tokuly.chat.v1.MessageService";

/**
 * A page that does what a hostile mini app would: asks for what its manifest
 * does not, sends broken requests and reaches for the page around it. It
 * writes each answer into `#log`; its button tries to leave for another origin.
 */
const HOSTILE = `<!doctype html><meta charset="utf-8"><title>hostile</title>
<pre id="log"></pre><button id="leave">leave</button>
<script>
const log = (line) => { document.getElementById("log").textContent += line + "\\n"; };
const ask = (call, id) => new Promise((resolve) => {
  const listener = (event) => {
    if (event.data && event.data.id === id) { removeEventListener("message", listener); resolve(event.data); }
  };
  addEventListener("message", listener);
  parent.postMessage({ protocol: "${PROTOCOL}", id, call }, "*");
});
(async () => {
  for (const [name, call] of [
    ["profile", { method: "getProfile" }],
    ["friends", { method: "pickFriends", max: 5 }],
    ["share", { method: "shareCard", card: { title: "hi", path: "/" } }],
    ["broken", { method: "shareCard", card: { title: "hi", path: "https://evil.example/" } }],
    ["unknown", { method: "getLocation" }],
  ]) {
    const answer = await ask(call, name);
    log(name + " " + (answer.ok ? "ok" : answer.error.code));
  }
  try { void parent.document.title; log("parent open"); } catch { log("parent closed"); }
  try { top.location.href = "https://example.com/"; log("top moved"); } catch { log("top stays"); }
  try { localStorage.setItem("x", "1"); log("storage open"); } catch { log("storage closed"); }
  log("done");
})();
document.getElementById("leave").onclick = () => { location.href = "http://127.0.0.2:9/"; };
</script>`;

/** An approved mini app of a fresh developer, at `path` of the sample server. */
async function approvedMiniApp(
  request: APIRequestContext,
  sample: SampleServer,
  { path, scopes }: { path: string; scopes: string[] },
): Promise<string> {
  const developer = await createAccount(request, "dev");
  const { app } = await rpc<{ app: { id: string } }>(
    request,
    "tokuly.dev.v1.AppService/CreateApp",
    {
      name: "割り勘",
      redirectUris: ["https://example.com/callback"],
      scopes: ["notes:read"],
      clientType: "APP_CLIENT_TYPE_PUBLIC",
    },
    developer,
  );
  let revision: number;
  try {
    const { review } = await rpc<{ review: { revision: number } }>(
      request,
      `${MINI_APPS}/SetMiniAppManifest`,
      { appId: app.id, name: "割り勘", launchUrl: `${sample.origin}${path}`, scopes },
      developer,
    );
    revision = review.revision;
  } catch (error) {
    if (!(error instanceof RpcError) || error.reason !== "LAUNCH_URL_INVALID") throw error;
    test.skip(
      true,
      "the API refuses loopback mini apps; run it with TOKULY_DEV_MINI_APP_ALLOW_LOOPBACK=true",
    );
    return "";
  }
  await rpc(request, `${MINI_APPS}/SubmitMiniApp`, { appId: app.id }, developer);
  const admin = await adminAccount(request);
  await rpc(
    request,
    "tokuly.dev.v1.MiniAppReviewService/ReviewMiniApp",
    { appId: app.id, revision, approve: true },
    admin,
  );
  return app.id;
}

async function directWith(request: APIRequestContext, as: Account, other: Account) {
  const { conversation } = await rpc<{ conversation: { id: string } }>(
    request,
    "tokuly.chat.v1.ConversationService/OpenDirect",
    { userId: other.userId },
    as,
  );
  return conversation.id;
}

const frameOf = (page: Page) => page.frameLocator('[data-testid="mini-app-frame"]');

test.describe("mini apps", () => {
  let sample: SampleServer;
  test.beforeEach(async ({ baseURL }) => {
    sample = await serveSample({
      hostOrigins: [new URL(baseURL ?? "").origin],
      pages: { "/hostile.html": HOSTILE },
    });
  });
  test.afterEach(async () => {
    await sample.close();
  });

  test("opens from a chat card, asks once, and shares a card back", async ({
    page,
    request,
    me,
    baseURL,
  }, testInfo) => {
    const appId = await approvedMiniApp(request, sample, {
      path: "/",
      scopes: ["profile", "friends"],
    });
    const yui = await createAccount(request, "ゆい");
    await befriend(request, me, yui);
    const conversationId = await directWith(request, me, yui);
    await rpc(
      request,
      `${MESSAGES}/SendMessage`,
      {
        conversationId,
        clientId: crypto.randomUUID(),
        text: `みんなで割り勘しよ!\n${new URL(baseURL ?? "").origin}/miniapps/${appId}`,
      },
      yui,
    );

    await page.goto(`/chat/${conversationId}`);
    const card = page.getByTestId("mini-app-card");
    await expect(card).toContainText("割り勘");
    await card.click();
    await expect(page).toHaveURL(/\/miniapps\/[^?]+\?c=.+&o=http/);

    // The sample asks who the person is as it starts: once, and only for what it declared.
    const consent = page.getByRole("dialog");
    await expect(consent).toContainText("「割り勘」に許可する?");
    await expect(consent.getByTestId("mini-app-scopes")).toContainText(
      "名前・ユーザー名・アイコン",
    );
    await page.screenshot({ path: testInfo.outputPath("consent.png") });
    await consent.getByRole("button", { name: "許可する" }).click();
    const frame = frameOf(page);
    await expect(frame.locator("#hello")).toHaveText(`こんにちは、${me.displayName}さん!`);
    await page.reload();
    await expect(frameOf(page).locator("#hello")).toHaveText(`こんにちは、${me.displayName}さん!`);
    await expect(page.getByRole("dialog")).toHaveCount(0);

    // Friends: only the ones the person picks.
    await frameOf(page).getByRole("button", { name: "友達をえらぶ" }).click();
    const picker = page.getByRole("dialog");
    await expect(picker).toContainText("友達をえらぼう");
    await picker.getByRole("button", { name: /ゆい/ }).click();
    await picker.getByRole("button", { name: "1人をわたす" }).click();
    await expect(frameOf(page).locator("#each")).toHaveText("¥6,000");
    await page.screenshot({ path: testInfo.outputPath("mini-app.png") });

    // A card goes to the chat only once the person sends it.
    await frameOf(page).getByRole("button", { name: "チャットに送る" }).click();
    const share = page.getByRole("dialog");
    await expect(share.getByTestId("mini-app-share-preview")).toContainText(
      "割り勘: ¥12,000 を2人で",
    );
    await page.screenshot({ path: testInfo.outputPath("share.png") });
    await share.getByRole("button", { name: "送る" }).click();
    await expect(frameOf(page).locator("#status")).toHaveText("チャットに送ったよ!");
    await expect
      .poll(async () => {
        const { messages = [] } = await rpc<{ messages?: { text: string }[] }>(
          request,
          `${MESSAGES}/ListMessages`,
          { conversationId, limit: 20 },
          yui,
        );
        return messages.map((message) => message.text).find((text) => text.startsWith("割り勘:"));
      })
      .toMatch(new RegExp(`/miniapps/${appId}\\?p=%2F%3Ftotal%3D12000%26n%3D2$`));

    // Back in the chat, the shared card opens the same split.
    await page.getByRole("link", { name: "もどる" }).click();
    await expect(page.getByTestId("mini-app-card")).toHaveCount(2);
    await page.screenshot({ path: testInfo.outputPath("chat.png") });
    await page.getByTestId("mini-app-card").last().click();
    await expect(frameOf(page).locator("#total")).toHaveValue("12000");
  });

  test("the bridge answers its own frame only, and only within the manifest", async ({
    page,
    request,
    me: _me,
  }) => {
    const appId = await approvedMiniApp(request, sample, { path: "/hostile.html", scopes: [] });
    await page.goto(`/miniapps/${appId}?o=${encodeURIComponent(sample.origin)}`);
    const log = frameOf(page).locator("#log");
    await expect(log).toContainText("done");
    expect((await log.textContent())?.trim().split("\n")).toEqual([
      "profile SCOPE_NOT_REQUESTED",
      "friends SCOPE_NOT_REQUESTED",
      "share NOT_IN_CONVERSATION",
      "broken INVALID_REQUEST",
      "unknown UNKNOWN_METHOD",
      "parent closed",
      "top stays",
      "storage closed",
      "done",
    ]);
    await expect(page.getByRole("dialog")).toHaveCount(0);

    // Requests from anything but the mini app's frame are not answered: this
    // window itself, and a frame of this origin put on the page.
    const answered = await page.evaluate(async (protocol) => {
      const request = { protocol, id: "forged", call: { method: "hello" } };
      const helper = document.createElement("iframe");
      document.body.append(helper);
      let answers = 0;
      const count = (event: MessageEvent) => {
        if (event.data?.id === "forged" && "ok" in event.data) answers += 1;
      };
      window.addEventListener("message", count);
      helper.contentWindow?.addEventListener("message", count);
      window.postMessage(request, "*");
      helper.contentWindow?.parent.postMessage(request, "*");
      await new Promise((resolve) => setTimeout(resolve, 500));
      return answers;
    }, PROTOCOL);
    expect(answered).toBe(0);

    // The frame cannot leave for an origin the page does not allow.
    await frameOf(page).getByRole("button", { name: "leave" }).click();
    await expect
      .poll(() => takePolicyViolations(/frame-src|127\.0\.0\.2/).length)
      .toBeGreaterThan(0);
  });

  test("a mini app that is not approved does not open", async ({ page, request, me: _me }) => {
    const developer = await createAccount(request, "dev");
    const { app } = await rpc<{ app: { id: string } }>(
      request,
      "tokuly.dev.v1.AppService/CreateApp",
      {
        name: "まだ",
        redirectUris: ["https://example.com/callback"],
        scopes: ["notes:read"],
        clientType: "APP_CLIENT_TYPE_PUBLIC",
      },
      developer,
    );
    await page.goto(`/miniapps/${app.id}?o=${encodeURIComponent(sample.origin)}`);
    await expect(page.getByRole("heading", { name: "このミニアプリは開けません" })).toBeVisible();
    await expect(page.getByTestId("mini-app-frame")).toHaveCount(0);
  });
});
