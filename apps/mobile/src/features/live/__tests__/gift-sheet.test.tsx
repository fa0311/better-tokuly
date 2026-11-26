import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen } from "@testing-library/react-native";
import { Code, ConnectError, create, live } from "@tokuly/api";
import { Alert } from "react-native";
import { SafeAreaProvider } from "react-native-safe-area-context";
import { api } from "#/lib/api";
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
      getBalance: async () => ({ balance: { coins: 5_000n } }),
      sendGift: (request: live.SendGiftRequest) => {
        sent.push(request);
        return sendGift(request);
      },
    }),
  };
  return { object, sent };
}

async function open() {
  await render(
    <SafeAreaProvider
      initialMetrics={{
        frame: { x: 0, y: 0, width: 320, height: 640 },
        insets: { top: 0, left: 0, right: 0, bottom: 0 },
      }}
    >
      <QueryClientProvider client={new QueryClient()}>
        <GiftSheet stream={stream} visible onClose={() => {}} />
      </QueryClientProvider>
    </SafeAreaProvider>,
  );
  await screen.findByTestId("gift-tier-3");
}

const confirm = async () => {
  await fireEvent.press(screen.getByTestId("gift-confirm"));
};

describe("GiftSheet", () => {
  beforeEach(() => jest.spyOn(Alert, "alert").mockImplementation(() => {}));

  it("repeats a gift whose answer was lost under its key, and nothing else", async () => {
    const a = account(async () => {
      throw new Error("the network went away");
    });
    jest.mocked(api).mockReturnValue(a.object as never);
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

  it("starts a new gift after a refusal: nothing was paid", async () => {
    const refusal = new ConnectError("short", Code.FailedPrecondition, {
      "x-tokuly-error": "INSUFFICIENT_COINS",
    });
    const a = account(async () => {
      throw refusal;
    });
    jest.mocked(api).mockReturnValue(a.object as never);
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
});
