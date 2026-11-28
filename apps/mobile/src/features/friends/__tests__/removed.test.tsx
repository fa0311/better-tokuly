import { QueryClientProvider } from "@tanstack/react-query";
import { act, render } from "@testing-library/react-native";
import { common, create, moments, shorts, social } from "@tokuly/api";
import { friendKeys } from "@tokuly/core/friends";
import { momentKeys } from "@tokuly/core/moments";
import { shortKeys } from "@tokuly/core/shorts";
import { testQueryClient } from "#/testing/query";
import FriendsBackground from "../background";

type Listener = (frame: unknown) => void;
const mockListeners = new Set<Listener>();

jest.mock("#/lib/realtime", () => ({
  realtime: () => ({
    connectionStatus: "connected",
    onFrame: (listener: Listener) => {
      mockListeners.add(listener);
      return () => mockListeners.delete(listener);
    },
    start: () => {},
  }),
}));

/** The server says a friendship with `userId` ended (an unfriend or a block, by either side). */
const friendRemoved = (userId: string) =>
  act(async () => {
    for (const listener of mockListeners)
      listener({
        topic: "",
        body: { case: "social", value: { event: { case: "friendRemovedUserId", value: userId } } },
      });
  });

const friends = (...ids: string[]) =>
  create(social.ListFriendsResponseSchema, {
    friends: ids.map((id) => ({ user: { id } })),
    total: ids.length,
  });

const feed = ["shorts", "feed", "following", "", 0, false];

/** The phone of someone friends with `aoi` and `ren`, with a feed and moments loaded. */
async function phone() {
  const queryClient = testQueryClient();
  queryClient.setQueryData(friendKeys.list(""), friends("aoi", "ren"));
  queryClient.setQueryData(feed, {
    pages: [
      {
        entries: [
          {
            short: create(shorts.ShortSchema, { id: "s1", visibility: common.Visibility.FRIENDS }),
          },
        ],
      },
    ],
    pageParams: [""],
  });
  queryClient.setQueryData(momentKeys.map, {
    moments: [
      create(moments.MomentSchema, { id: "m1", author: { id: "aoi" } }),
      create(moments.MomentSchema, { id: "m2", author: { id: "ren" } }),
    ],
  });
  await render(
    <QueryClientProvider client={queryClient}>
      <FriendsBackground />
    </QueryClientProvider>,
  );
  return queryClient;
}

afterEach(() => mockListeners.clear());

describe("a friendship the other side ended", () => {
  it("takes them out of the friends list and the moments", async () => {
    const queryClient = await phone();
    await friendRemoved("aoi");
    const left = queryClient.getQueryData<social.ListFriendsResponse>(friendKeys.list(""));
    expect(left?.friends.map((friend) => friend.user?.id)).toEqual(["ren"]);
    const map = queryClient.getQueryData<moments.ListMapMomentsResponse>(momentKeys.map);
    expect(map?.moments.map((moment) => moment.id)).toEqual(["m2"]);
  });

  it("stops the shorts feed from playing their friends-only shorts (#529)", async () => {
    const queryClient = await phone();
    await friendRemoved("aoi");
    expect(queryClient.getQueryData(feed)).toBeUndefined();
    expect(queryClient.getQueriesData({ queryKey: shortKeys.all })).toEqual([[feed, undefined]]);
  });
});
