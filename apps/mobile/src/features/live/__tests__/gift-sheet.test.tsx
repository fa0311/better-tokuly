import { QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen } from "@testing-library/react-native";
import { Code, ConnectError, create, live } from "@tokuly/api";
import { Alert } from "react-native";
import { SafeAreaProvider } from "react-native-safe-area-context";
import { useSessionStore } from "#/features/session/store";
import { api } from "#/lib/api";
import { testQueryClient } from "#/testing/query";
import { GiftSheet } from "../gift-sheet";

jest.mock("#/lib/api", () => ({ api: jest.fn(), apiWith: jest.fn(), authClient: jest.fn() }));
jest.mock("expo-router", () => ({ router: { push: jest.fn() } }));
let mockUuid = 0;
jest.mock("expo-crypto", () => ({ randomUUID: () => `uuid-${++mockUuid}-abcdefgh` }));

const TIERS = [
  create(live.GiftTierSchema, { level: 1, minCoins: 10n, maxMessageLength: 0 }),
  create(live.GiftTierSchema, { level: 3, minCoins: 100n, maxMessageLength: 50 }),
];

const stream = create(live.StreamSchema, {
  summary: { id: "s1", shortId: "abc", status: live.StreamStatus.LIVE, channel: { name: "ミナ" } },
  giftsEnabled: true,
});

/** The API of one account: the gift ladder, a balance, and the gifts it was asked to send. */
function account(sendGift: (request: live.SendGiftRequest) => Promise<unknown>) {
  const sent: live.SendGiftRequest[] = [];
  const object = {
    service: () => ({
      getGiftTiers: async () => ({ tiers: TIERS, minCoins: 10n, maxCoins: 100_000n }),
      getBalance: async () => ({ balance: { coins: 54_321n } }),
      sendGift: (request: live.SendGiftRequest) => {
        sent.push(request);
        return sendGift(request);
      },
    }),
  };
  return { object, sent };
}

/** Makes `who` the account in use, as a sign-in or a switch does. */
function signIn(who: ReturnType<typeof account>, userId: string) {
  jest.mocked(api).mockReturnValue(who.object as never);
  useSessionStore.setState({
    state: { status: "signedIn", sessionId: `session-${userId}`, me: { user: { id: userId } } },
  } as never);
}

const lostAnswer = async () => {
  throw new Error("the network went away");
};

/** Opens the sheet; `step` is what it shows first (the confirmation, when a gift is unanswered). */
async function open(step = "gift-tier-3") {
  await render(
    <SafeAreaProvider
      initialMetrics={{
        frame: { x: 0, y: 0, width: 320, height: 640 },
        insets: { top: 0, left: 0, right: 0, bottom: 0 },
      }}
    >
      <QueryClientProvider client={testQueryClient()}>
        <GiftSheet stream={stream} visible onClose={() => {}} />
      </QueryClientProvider>
    </SafeAreaProvider>,
  );
  await screen.findByTestId(step);
}

const confirm = async () => {
  await fireEvent.press(screen.getByTestId("gift-confirm"));
};

describe("GiftSheet", () => {
  beforeEach(() => jest.spyOn(Alert, "alert").mockImplementation(() => {}));

  it("repeats a gift whose answer was lost under its key, and nothing else", async () => {
    const a = account(lostAnswer);
    signIn(a, "u1");
    await open();
    await fireEvent.changeText(screen.getByTestId("gift-message"), "がんばれ！");
    await fireEvent.press(screen.getByTestId("gift-next"));
    await confirm();
    // The answer was lost: the same gift again keeps its key, so the server finishes that one.
    expect(await screen.findByText(/二重には贈られません/)).toBeOnTheScreen();
    await confirm();
    expect(a.sent).toHaveLength(2);
    expect(a.sent[1]?.idempotencyKey).toBe(a.sent[0]?.idempotencyKey);
    expect(a.sent[0]).toMatchObject({ streamId: "s1", coins: 100n, message: "がんばれ！" });

    // Going back and changing the gift makes it a new one.
    await fireEvent.press(screen.getByText("もどる"));
    await fireEvent.press(screen.getByTestId("gift-tier-1"));
    await fireEvent.press(screen.getByTestId("gift-next"));
    await confirm();
    expect(a.sent).toHaveLength(3);
    expect(a.sent[2]?.idempotencyKey).not.toBe(a.sent[0]?.idempotencyKey);
    expect(a.sent[2]).toMatchObject({ coins: 10n, message: "" });
  });

  it("shows the amount that is paid exactly, never rounded", async () => {
    const a = account(lostAnswer);
    signIn(a, "u9");
    await open();
    expect(await screen.findByText("🪙 54,321")).toBeOnTheScreen();
    await fireEvent.changeText(screen.getByTestId("gift-amount"), "12345");
    await fireEvent.press(screen.getByTestId("gift-next"));
    expect(screen.getByText("12,345コインを贈る")).toBeOnTheScreen();
  });

  it("starts a new gift after a refusal: nothing was paid", async () => {
    const refusal = new ConnectError("short", Code.FailedPrecondition, {
      "x-tokuly-error": "INSUFFICIENT_COINS",
    });
    const a = account(async () => {
      throw refusal;
    });
    signIn(a, "u2");
    await open();
    await fireEvent.press(screen.getByTestId("gift-next"));
    await confirm();
    expect(a.sent).toHaveLength(1);
    // Back on the first step; the next confirmation is a new gift with a new key.
    await fireEvent.press(await screen.findByTestId("gift-next"));
    expect(screen.queryByText(/二重には贈られません/)).not.toBeOnTheScreen();
    await confirm();
    expect(a.sent).toHaveLength(2);
    expect(a.sent[1]?.idempotencyKey).not.toBe(a.sent[0]?.idempotencyKey);
  });

  it("keeps the key of a gift whose answer was lost through going back and closing the app", async () => {
    const a = account(lostAnswer);
    signIn(a, "u3");
    await open();
    await fireEvent.press(screen.getByTestId("gift-tier-1"));
    await fireEvent.press(screen.getByTestId("gift-next"));
    await confirm();
    // Back and on again with the same gift: still the gift that may have been paid.
    await fireEvent.press(await screen.findByText("もどる"));
    await fireEvent.press(screen.getByTestId("gift-next"));
    expect(screen.getByText(/二重には贈られません/)).toBeOnTheScreen();
    await confirm();
    expect(a.sent).toHaveLength(2);
    expect(a.sent[1]?.idempotencyKey).toBe(a.sent[0]?.idempotencyKey);

    // The app is closed and opened again: the sheet comes back with that gift, under its key.
    await screen.unmount();
    await open("gift-confirm");
    expect(await screen.findByText(/二重には贈られません/)).toBeOnTheScreen();
    await confirm();
    expect(a.sent).toHaveLength(3);
    expect(a.sent[2]).toMatchObject({ coins: 10n, idempotencyKey: a.sent[0]?.idempotencyKey });
  });

  it("forgets a gift once the server answered, and never repeats another account's", async () => {
    let answer: (request: live.SendGiftRequest) => Promise<unknown> = lostAnswer;
    const a = account((request) => answer(request));
    signIn(a, "u4");
    await open();
    await fireEvent.press(screen.getByTestId("gift-next"));
    await confirm();
    await screen.findByText(/二重には贈られません/);

    // Another account on this phone, the same stream and the same gift: its own gift, its own key.
    await screen.unmount();
    const b = account(lostAnswer);
    signIn(b, "u5");
    await open();
    expect(screen.queryByText(/二重には贈られません/)).not.toBeOnTheScreen();
    await fireEvent.press(screen.getByTestId("gift-next"));
    await confirm();
    expect(b.sent[0]?.idempotencyKey).not.toBe(a.sent[0]?.idempotencyKey);

    // Back as the first account, the repeat is answered: the next gift is a new one.
    await screen.unmount();
    signIn(a, "u4");
    answer = async (request) => ({ gift: { coins: request.coins, message: request.message } });
    await open("gift-confirm");
    await screen.findByText(/二重には贈られません/);
    await confirm();
    expect(a.sent[1]?.idempotencyKey).toBe(a.sent[0]?.idempotencyKey);
    await screen.unmount();
    await open();
    expect(screen.queryByText(/二重には贈られません/)).not.toBeOnTheScreen();
    await fireEvent.press(screen.getByTestId("gift-next"));
    await confirm();
    expect(a.sent[2]?.idempotencyKey).not.toBe(a.sent[0]?.idempotencyKey);
  });

  it("sends one gift when the confirmation is pressed twice at once", async () => {
    const a = account(() => new Promise(() => {}));
    signIn(a, "u6");
    await open();
    await fireEvent.press(screen.getByTestId("gift-next"));
    const button = screen.getByTestId("gift-confirm");
    await Promise.all([fireEvent.press(button), fireEvent.press(button)]);
    await screen.findByTestId("gift-sheet");
    expect(a.sent).toHaveLength(1);
  });
});
