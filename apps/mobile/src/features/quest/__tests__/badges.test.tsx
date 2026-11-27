import { QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen, waitFor } from "@testing-library/react-native";
import { create, identity, quest } from "@tokuly/api";
import { router } from "expo-router";
import type { ReactNode } from "react";
import { Alert } from "react-native";
import { SafeAreaProvider } from "react-native-safe-area-context";
import { api } from "#/lib/api";
import { testQueryClient } from "#/testing/query";
import { BadgeScreen } from "../badge-screen";
import { BadgesScreen } from "../badges-screen";
import profile from "../profile";

jest.mock("#/lib/api", () => ({ api: jest.fn(), apiWith: jest.fn(), authClient: jest.fn() }));
// The one account signed in all along.
jest.mock("#/features/session/account", () => ({
  currentAccount: () => ({
    sessionId: "s1",
    userId: "u1",
    api: jest.requireMock("#/lib/api").api(),
  }),
  isActive: () => true,
}));
jest.mock("#/features/session/store", () => ({ useMe: () => ({ timeZone: "Asia/Tokyo" }) }));
jest.mock("expo-router", () => ({ router: { push: jest.fn() }, Stack: { Screen: () => null } }));

const gold = create(quest.BadgeSchema, {
  key: "chatter_gold",
  line: "chatter",
  tier: quest.BadgeTier.GOLD,
  name: { ja: "おしゃべり", en: "Chatter" },
  description: { ja: "メッセージを1000こ送る", en: "Send 1000 messages" },
  emoji: "💬",
  title: { ja: "おしゃべり王", en: "Chat King" },
  owned: true,
  earnedAt: { seconds: 1_790_000_000n, nanos: 0 },
});
const bronze = create(quest.BadgeSchema, {
  ...gold,
  key: "chatter_bronze",
  tier: quest.BadgeTier.BRONZE,
});
const secret = create(quest.BadgeSchema, { key: "secret_1", secret: true });

/**
 * The badge service with what the caller shows; the showcases and titles it
 * was sent. `mineFails`: the caller's own badges cannot be read.
 */
function server({ mineFails = false } = {}) {
  const sent: { showcase: string[][]; title: string[] } = { showcase: [], title: [] };
  jest.mocked(api).mockReturnValue({
    service: () => ({
      listBadgeCatalog: async () =>
        create(quest.ListBadgeCatalogResponseSchema, {
          families: [
            {
              key: "chat",
              name: { ja: "チャット" },
              emoji: "💬",
              badges: [bronze, gold, secret],
              ownedCount: 2,
            },
          ],
          ownedCount: 2,
          totalCount: 3,
        }),
      listUserBadges: async () => {
        if (mineFails) throw new Error("the connection dropped");
        return create(quest.ListUserBadgesResponseSchema, {
          badges: [gold, bronze],
          showcase: [bronze],
        });
      },
      getBadge: async () =>
        create(quest.GetBadgeResponseSchema, {
          badge: gold,
          friendOwners: [{ id: "u2", handle: "hana", displayName: "はな" }],
          friendOwnerCount: 3,
        }),
      setShowcase: async ({ badgeKeys }: { badgeKeys: string[] }) => {
        sent.showcase.push(badgeKeys);
        return {};
      },
      setTitle: async ({ badgeKey }: { badgeKey: string }) => {
        sent.title.push(badgeKey);
        return {};
      },
    }),
  } as never);
  return sent;
}

async function show(children: ReactNode) {
  await render(
    <SafeAreaProvider
      initialMetrics={{
        frame: { x: 0, y: 0, width: 320, height: 640 },
        insets: { top: 0, left: 0, right: 0, bottom: 0 },
      }}
    >
      <QueryClientProvider
        client={testQueryClient({ defaultOptions: { queries: { retry: false } } })}
      >
        {children}
      </QueryClientProvider>
    </SafeAreaProvider>,
  );
}

describe("badges", () => {
  beforeEach(() => jest.spyOn(Alert, "alert").mockImplementation(() => {}));
  afterEach(() => jest.clearAllMocks());

  it("lists the collection by family, tiers side by side, secrets as secrets", async () => {
    server();
    await show(<BadgesScreen />);
    expect((await screen.findByTestId("quest-badge-count")).props.children).toBe("2 / 3 こ");
    expect(screen.getByText("ゲットするまでひみつ!")).toBeTruthy();
    await fireEvent.press(screen.getByTestId("badge-chatter_gold"));
    expect(router.push).toHaveBeenCalledWith("/quest/badges/chatter_gold");
  });

  it("puts an earned badge on the profile next to the ones shown, and wears its title", async () => {
    const sent = server();
    await show(<BadgeScreen badgeKey="chatter_gold" />);
    expect(await screen.findByText("おしゃべり(金)")).toBeTruthy();
    expect(screen.getByText("ほか2人")).toBeTruthy();
    await waitFor(() => expect(screen.getByText("プロフィールにかざる")).toBeTruthy());
    await fireEvent.press(screen.getByTestId("quest-badge-showcase"));
    await fireEvent.press(screen.getByTestId("quest-badge-title"));
    await waitFor(() => expect(sent.title).toEqual(["chatter_gold"]));
    expect(sent.showcase).toEqual([["chatter_bronze", "chatter_gold"]]);
  });

  it("never sends a showcase while the one shown now is unknown", async () => {
    const sent = server({ mineFails: true });
    await show(<BadgeScreen badgeKey="chatter_gold" />);
    await waitFor(() => expect(screen.getByText("プロフィールにかざる")).toBeTruthy());
    await new Promise((resolve) => setTimeout(resolve, 50));
    await fireEvent.press(screen.getByTestId("quest-badge-showcase"));
    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(sent.showcase).toEqual([]);
  });
});

describe("the quest card on a profile", () => {
  const Card = profile[0]?.Component as NonNullable<(typeof profile)[0]>["Component"];
  const person = (cards: { kind: string; data: Record<string, string> }[]) => {
    const user = { id: "u2", handle: "hana", displayName: "はな" };
    return { profile: create(identity.ProfileSchema, { user, cards }), user: user as never };
  };

  it("shows a friend's level, title, streak and showcase", async () => {
    const { profile: p, user } = person([
      {
        kind: "quest.level",
        data: {
          level: "12",
          title_ja: "旅人",
          badges: JSON.stringify([
            { key: "chatter_gold", emoji: "💬", tier: 3, name_ja: "おしゃべり" },
          ]),
        },
      },
      { kind: "quest.streak", data: { days: "7", state: "1" } },
    ]);
    await show(<Card profile={p} user={user} isSelf={false} isFriend />);
    expect(screen.getByText("Lv.12")).toBeTruthy();
    expect(screen.getByText("👑 旅人")).toBeTruthy();
    expect(screen.getByText(/7日連続/)).toBeTruthy();
    await fireEvent.press(screen.getByTestId("quest-profile-badge-chatter_gold"));
    expect(router.push).toHaveBeenCalledWith("/quest/badges/chatter_gold");
  });

  it("shows nothing for someone who keeps their progress to themselves", async () => {
    const { profile: p, user } = person([]);
    await show(<Card profile={p} user={user} isSelf={false} isFriend={false} />);
    expect(screen.queryByTestId("quest-profile")).toBeNull();
  });
});
