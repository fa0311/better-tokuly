/**
 * 割り勘: split a bill among friends picked from Tokuly, and send the result
 * back to the chat as a card that opens the same split again.
 */
import { BridgeError, connect, type MiniApp, type Person } from "@tokuly/miniapp";

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;
const yen = (amount: number) => `¥${amount.toLocaleString("ja-JP")}`;

const total = $<HTMLInputElement>("total");
const status = $("status");
let me: Person | undefined;
let friends: Person[] = [];
/** How many people a card that opened this page split among. */
let sharedCount = 0;

function people(): Person[] {
  return me ? [me, ...friends] : friends;
}

function count(): number {
  return Math.max(1, people().length || sharedCount);
}

function amount(): number {
  return Math.max(0, Math.floor(Number(total.value.replace(/[^0-9]/g, "")) || 0));
}

function render(): void {
  $("each").textContent = yen(Math.ceil(amount() / count()));
  const list = $("people");
  list.replaceChildren(
    ...people().map((person) => {
      const item = document.createElement("li");
      const face = person.avatarUrl
        ? document.createElement("img")
        : document.createElement("span");
      face.className = "dot";
      if (face instanceof HTMLImageElement) face.src = person.avatarUrl;
      item.append(face, person.name);
      return item;
    }),
  );
}

function say(text: string): void {
  status.textContent = text;
}

/** A refusal in words; the person closing a sheet is no news. */
function explain(error: unknown): void {
  if (error instanceof BridgeError && error.code === "CANCELLED") return;
  if (error instanceof BridgeError && error.code === "CONSENT_DENIED") say("許可されなかったよ");
  else say(error instanceof Error ? error.message : String(error));
}

async function start(tokuly: MiniApp): Promise<void> {
  const query = new URLSearchParams(tokuly.launch.path.split("?")[1] ?? "");
  if (query.has("total")) total.value = query.get("total") ?? "";
  sharedCount = Number(query.get("n")) || 0;
  render();
  $<HTMLButtonElement>("share").disabled = !tokuly.launch.inConversation;
  $("hello").textContent = "だれと割る?";
  try {
    me = await tokuly.getProfile();
    $("hello").textContent = `こんにちは、${me.name}さん!`;
    render();
  } catch (error) {
    explain(error);
  }
  $("pick").addEventListener("click", async () => {
    try {
      friends = await tokuly.pickFriends({ max: 9 });
      render();
    } catch (error) {
      explain(error);
    }
  });
  $("share").addEventListener("click", async () => {
    const names = people()
      .map((person) => person.name)
      .join("・");
    try {
      await tokuly.shareCard({
        title: `割り勘: ${yen(amount())} を${count()}人で`,
        text: `1人 ${yen(Math.ceil(amount() / count()))}${names ? `（${names}）` : ""}`,
        path: `/?total=${amount()}&n=${count()}`,
      });
      say("チャットに送ったよ!");
    } catch (error) {
      explain(error);
    }
  });
}

total.addEventListener("input", render);
const hosts = document.querySelector<HTMLMetaElement>('meta[name="tokuly-hosts"]')?.content ?? "";
try {
  await start(await connect({ hostOrigins: hosts.split(",").filter(Boolean) }));
} catch (error) {
  $("hello").textContent =
    error instanceof BridgeError && error.code === "NOT_IN_TOKULY"
      ? "Tokulyのチャットから開いてね"
      : "Tokulyにつながりませんでした";
  render();
}
