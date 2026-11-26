import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen, waitFor } from "@testing-library/react-native";
import { community, create } from "@tokuly/api";
import { router } from "expo-router";
import type { ReactNode } from "react";
import { Alert } from "react-native";
import { SafeAreaProvider } from "react-native-safe-area-context";
import { currentAccount, isActive } from "#/features/session/account";
import { api } from "#/lib/api";
import { realtime } from "#/lib/realtime";
import { CommunityScreen } from "../community-screen";
import { ComposeThreadScreen } from "../compose-screen";

jest.mock("#/lib/api", () => ({ api: jest.fn(), apiWith: jest.fn(), authClient: jest.fn() }));
jest.mock("#/lib/realtime", () => ({ realtime: jest.fn() }));
jest.mock("#/features/session/account", () => ({ currentAccount: jest.fn(), isActive: jest.fn() }));
jest.mock("expo-router", () => ({
  router: { push: jest.fn(), back: jest.fn() },
  Stack: { Screen: () => null },
}));

/**
 * One account's community service whose writes are answered only after the
 * person switched to another account: what they were sent, and the switch.
 */
function switchingAccount() {
  let switched = false;
  const sent: string[] = [];
  const neko = create(community.CommunitySchema, {
    id: "c1",
    handle: "neko",
    name: "ねこ部",
    visibility: community.CommunityVisibility.PUBLIC,
    viewer: { status: community.MembershipStatus.NONE },
  });
  const answerAfterSwitch = <T,>(name: string, answer: T) => {
    sent.push(name);
    switched = true;
    return Promise.resolve(answer);
  };
  const client = {
    service: () => ({
      getCommunity: async () => ({ community: neko }),
      listMyCommunities: async () => ({ communities: [], requested: [] }),
      listChannels: async () => ({
        channels: [create(community.ChannelSchema, { id: "ch1", name: "雑談", canPost: true })],
      }),
      listThreads: async () => ({ pinned: [], threads: [] }),
      listAnnouncements: async () => ({ threads: [] }),
      joinCommunity: () =>
        answerAfterSwitch("join", { status: community.MembershipStatus.MEMBER, community: neko }),
      createThread: () =>
        answerAfterSwitch("thread", {
          thread: create(community.ThreadSchema, { id: "t1", shortId: "abc", communityId: "c1" }),
        }),
    }),
  };
  jest.mocked(api).mockReturnValue(client as never);
  jest
    .mocked(currentAccount)
    .mockReturnValue({ sessionId: "s1", userId: "u1", api: client as never });
  jest.mocked(isActive).mockImplementation(() => !switched);
  jest.mocked(realtime).mockReturnValue({
    subscribe: () => ({ release: () => {} }),
    start: () => {},
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
      <QueryClientProvider client={new QueryClient()}>{children}</QueryClientProvider>
    </SafeAreaProvider>,
  );
}

describe("a community request answered after an account switch", () => {
  beforeEach(() => jest.spyOn(Alert, "alert").mockImplementation(() => {}));
  afterEach(() => jest.clearAllMocks());

  it("does not welcome the next account into the community", async () => {
    const sent = switchingAccount();
    await show(<CommunityScreen handle="neko" />);
    await fireEvent.press(await screen.findByTestId("community-join"));
    await waitFor(() => expect(sent).toEqual(["join"]));
    expect(Alert.alert).not.toHaveBeenCalled();
  });

  it("does not open the new thread under the next account", async () => {
    const sent = switchingAccount();
    await show(<ComposeThreadScreen handle="neko" />);
    await fireEvent.changeText(await screen.findByTestId("compose-title"), "はじめまして！");
    await fireEvent.changeText(screen.getByTestId("compose-body"), "よろしくね");
    await fireEvent.press(screen.getByTestId("compose-post"));
    await waitFor(() => expect(sent).toEqual(["thread"]));
    expect(router.back).not.toHaveBeenCalled();
    expect(router.push).not.toHaveBeenCalled();
  });
});
