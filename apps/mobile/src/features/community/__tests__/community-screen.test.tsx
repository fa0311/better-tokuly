import { QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen, waitFor } from "@testing-library/react-native";
import { community, create } from "@tokuly/api";
import { SafeAreaProvider } from "react-native-safe-area-context";
import { api } from "#/lib/api";
import { realtime } from "#/lib/realtime";
import { testQueryClient } from "#/testing/query";
import { CommunityScreen } from "../community-screen";

jest.mock("#/lib/api", () => ({ api: jest.fn(), apiWith: jest.fn(), authClient: jest.fn() }));
// The one account signed in all along.
jest.mock("#/features/session/account", () => ({
  currentAccount: () => ({ sessionId: "s1", userId: "u1", api: {} }),
  isActive: () => true,
}));
jest.mock("#/lib/realtime", () => ({ realtime: jest.fn() }));
jest.mock("expo-router", () => ({ router: { push: jest.fn() }, Stack: { Screen: () => null } }));

const S = community.MembershipStatus;

/** The community domain as the server answers it, with who joined and which topics were followed. */
function server(visibility: community.CommunityVisibility) {
  let status = S.NONE;
  const joined: string[] = [];
  const topics: string[] = [];
  const neko = () =>
    create(community.CommunitySchema, {
      id: "c1",
      handle: "neko",
      name: "ねこ部",
      visibility,
      memberCount: 3,
      viewer: { status },
    });
  jest.mocked(realtime).mockReturnValue({
    subscribe: (topic: string) => {
      topics.push(topic);
      return { release: () => {} };
    },
    start: () => {},
  } as never);
  jest.mocked(api).mockReturnValue({
    service: () => ({
      getCommunity: async () => ({ community: neko() }),
      joinCommunity: async ({ communityId }: { communityId: string }) => {
        joined.push(communityId);
        status = visibility === community.CommunityVisibility.PUBLIC ? S.MEMBER : S.REQUESTED;
        return { status, community: neko() };
      },
      listMyCommunities: async () => ({ communities: [], requested: [] }),
      listChannels: async () => ({
        channels: [
          create(community.ChannelSchema, {
            id: "ch1",
            name: "雑談",
            canPost: status === S.MEMBER,
          }),
        ],
      }),
      listThreads: async () => ({
        pinned: [],
        threads: [
          create(community.ThreadSchema, {
            id: "t1",
            shortId: "abc",
            communityId: "c1",
            title: "はじめまして！",
            replyCount: 2,
          }),
        ],
      }),
      listAnnouncements: async () => ({ threads: [] }),
    }),
  } as never);
  return { joined, topics };
}

async function show() {
  await render(
    <SafeAreaProvider
      initialMetrics={{
        frame: { x: 0, y: 0, width: 320, height: 640 },
        insets: { top: 0, left: 0, right: 0, bottom: 0 },
      }}
    >
      <QueryClientProvider client={testQueryClient()}>
        <CommunityScreen handle="neko" />
      </QueryClientProvider>
    </SafeAreaProvider>,
  );
}

describe("CommunityScreen", () => {
  it("shows a public community's board, follows its topic, and joins it", async () => {
    const { joined, topics } = server(community.CommunityVisibility.PUBLIC);
    await show();
    expect(await screen.findByText("はじめまして！")).toBeOnTheScreen();
    expect(topics).toEqual(["community:c1"]);
    expect(screen.queryByTestId("thread-new")).not.toBeOnTheScreen();
    await fireEvent.press(screen.getByTestId("community-join"));
    await waitFor(() => expect(joined).toEqual(["c1"]));
    expect(await screen.findByTestId("community-joined")).toBeOnTheScreen();
    expect(await screen.findByTestId("thread-new")).toBeOnTheScreen();
  });

  it("keeps a restricted community behind its front door and asks to join", async () => {
    const { joined, topics } = server(community.CommunityVisibility.RESTRICTED);
    await show();
    expect(await screen.findByTestId("community-front-door")).toBeOnTheScreen();
    expect(screen.queryByText("はじめまして！")).not.toBeOnTheScreen();
    expect(topics).toEqual([]);
    await fireEvent.press(screen.getByTestId("community-join"));
    await fireEvent.press(await screen.findByTestId("join-request-send"));
    await waitFor(() => expect(joined).toEqual(["c1"]));
    expect(await screen.findByTestId("community-cancel-request")).toBeOnTheScreen();
  });
});
