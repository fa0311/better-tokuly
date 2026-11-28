import type { Client } from "@connectrpc/connect";
import { InfiniteQueryObserver, QueryClient } from "@tanstack/query-core";
import { create, type MessageInitShape, moments } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import { make } from "../testing/make";
import {
  applyMomentsEvent,
  forgetUserMoments,
  rememberPostedMoment,
  removeMoment,
  setMyReaction,
} from "./cache";
import { momentKeys } from "./keys";
import { momentQueries } from "./queries";

const user = (id: string) => ({ id, handle: id, displayName: id });
const moment = (
  id: string,
  author: string,
  extra: MessageInitShape<typeof moments.MomentSchema> = {},
) => make(moments.MomentSchema, { id, author: user(author) }, extra);
const reaction = (id: string, by: string) =>
  create(moments.MomentReactionSchema, { id, user: user(by) });

function withCache(
  friends: moments.Moment[],
  today: MessageInitShape<typeof moments.GetTodayResponseSchema> = {},
) {
  const queryClient = new QueryClient();
  queryClient.setQueryData(momentKeys.friends, {
    pages: [create(moments.ListFriendMomentsResponseSchema, { moments: friends })],
    pageParams: [""],
  });
  queryClient.setQueryData(
    momentKeys.map,
    create(moments.ListMapMomentsResponseSchema, { moments: friends }),
  );
  queryClient.setQueryData(momentKeys.today, create(moments.GetTodayResponseSchema, today));
  return queryClient;
}

const friendIds = (queryClient: QueryClient) =>
  queryClient
    .getQueryData<{ pages: moments.ListFriendMomentsResponse[] }>(momentKeys.friends)
    ?.pages.flatMap((page) => page.moments.map((entry) => entry.id));
const mapIds = (queryClient: QueryClient) =>
  queryClient
    .getQueryData<moments.ListMapMomentsResponse>(momentKeys.map)
    ?.moments.map((entry) => entry.id);
const today = (queryClient: QueryClient) =>
  queryClient.getQueryData<moments.GetTodayResponse>(momentKeys.today);

describe("moment cache", () => {
  it("unlocks the day once the caller posted", () => {
    const queryClient = withCache([moment("a1", "aoi", { locked: true })]);
    rememberPostedMoment(queryClient, moment("m1", "me", { mine: true }));
    expect(today(queryClient)).toMatchObject({ posted: true, unlocked: true, mine: { id: "m1" } });
    expect(queryClient.getQueryState(momentKeys.friends)?.isInvalidated).toBe(true);
  });

  it("locks again when the caller's own moment of the day goes, and keeps the day used", () => {
    const mine = moment("m1", "me", { mine: true });
    const queryClient = withCache([moment("a1", "aoi")], { mine, posted: true, unlocked: true });
    removeMoment(queryClient, "m1");
    expect(today(queryClient)).toMatchObject({ posted: true, unlocked: false, mine: undefined });
    expect(friendIds(queryClient)).toEqual(["a1"]);
  });

  it("takes a deleted friend's moment off the list and the map", () => {
    const queryClient = withCache([moment("a1", "aoi"), moment("r1", "ren")]);
    applyMomentsEvent(queryClient, {
      case: "deleted",
      value: create(moments.MomentRefSchema, { momentId: "a1", authorId: "aoi" }),
    });
    expect(friendIds(queryClient)).toEqual(["r1"]);
    expect(mapIds(queryClient)).toEqual(["r1"]);
  });

  it("replaces the caller's reaction and keeps the count", () => {
    const before = reaction("x1", "me");
    const queryClient = withCache([
      moment("a1", "aoi", {
        myReaction: before,
        reactions: [before, reaction("y1", "ren")],
        reactionCount: 2,
      }),
    ]);
    setMyReaction(queryClient, "a1", reaction("x2", "me"));
    const [entry] =
      queryClient.getQueryData<moments.ListMapMomentsResponse>(momentKeys.map)?.moments ?? [];
    expect(entry?.reactions.map((item) => item.id)).toEqual(["x2", "y1"]);
    expect(entry?.reactionCount).toBe(2);
    setMyReaction(queryClient, "a1", undefined);
    expect(friendIds(queryClient)).toEqual(["a1"]);
    const after = queryClient.getQueryData<moments.ListMapMomentsResponse>(momentKeys.map);
    expect(after?.moments[0]).toMatchObject({ reactionCount: 1, myReaction: undefined });
  });

  it("counts reactions the owner was told about on their own moment", () => {
    const mine = moment("m1", "me", { mine: true, reactionCount: 1 });
    const queryClient = withCache([], { mine, posted: true });
    applyMomentsEvent(queryClient, {
      case: "reactionsChanged",
      value: create(moments.ReactionsChangedSchema, { momentId: "m1", reactionCount: 3 }),
    });
    expect(today(queryClient)?.mine?.reactionCount).toBe(3);
  });

  it("starts a new day locked when the prompt arrives", () => {
    const queryClient = withCache([], { posted: true, unlocked: true, mine: moment("m0", "me") });
    const day = create(moments.MomentDaySchema, { date: "2026-11-29" });
    applyMomentsEvent(queryClient, { case: "prompt", value: day });
    expect(today(queryClient)).toMatchObject({
      day: { date: "2026-11-29" },
      posted: false,
      unlocked: false,
      mine: undefined,
    });
  });

  it("forgets a person's moments after an unfriend or a block", () => {
    const queryClient = withCache([moment("a1", "aoi"), moment("r1", "ren")]);
    queryClient.setQueryData(momentKeys.moment("a1"), moment("a1", "aoi"));
    queryClient.setQueryData(momentKeys.moment("a2"), moment("a2", "aoi"));
    queryClient.setQueryData(momentKeys.reactions("a1"), { reactions: [reaction("x1", "ren")] });
    queryClient.setQueryData(momentKeys.reactions("a2"), { reactions: [reaction("x2", "ren")] });
    queryClient.setQueryData(momentKeys.reactions("r1"), { reactions: [reaction("x3", "aoi")] });
    forgetUserMoments(queryClient, "aoi");
    expect(friendIds(queryClient)).toEqual(["r1"]);
    expect(mapIds(queryClient)).toEqual(["r1"]);
    expect(queryClient.getQueryData(momentKeys.moment("a1"))).toBeUndefined();
    expect(queryClient.getQueryData(momentKeys.moment("a2"))).toBeUndefined();
    expect(queryClient.getQueryData(momentKeys.reactions("a1"))).toBeUndefined();
    expect(queryClient.getQueryData(momentKeys.reactions("a2"))).toBeUndefined();
    expect(queryClient.getQueryData(momentKeys.reactions("r1"))).toBeDefined();
  });
});

/** A server whose friends' list and map the test hands out, one answer per read. */
function server() {
  const lists: ((list: moments.Moment[], nextCursor?: string) => void)[] = [];
  const maps: ((list: moments.Moment[]) => void)[] = [];
  const client = () =>
    ({
      listFriendMoments: () =>
        new Promise((resolve) =>
          lists.push((list, nextCursor = "") =>
            resolve(
              create(moments.ListFriendMomentsResponseSchema, {
                moments: list,
                page: { nextCursor },
              }),
            ),
          ),
        ),
      listMapMoments: () =>
        new Promise((resolve) =>
          maps.push((list) =>
            resolve(create(moments.ListMapMomentsResponseSchema, { moments: list })),
          ),
        ),
    }) as unknown as Client<typeof moments.MomentService>;
  return {
    client,
    list: (read: number, list: moments.Moment[], nextCursor?: string) =>
      lists[read]?.(list, nextCursor),
    map: (read: number, list: moments.Moment[]) => maps[read]?.(list),
  };
}

describe("a read in flight when a moment or a person goes", () => {
  it("does not bring a deleted moment back to the friends' list or the map", async () => {
    const queryClient = new QueryClient();
    const api = server();
    const list = queryClient.fetchInfiniteQuery(momentQueries.friends(api.client));
    const map = queryClient.fetchQuery(momentQueries.map(api.client));
    applyMomentsEvent(queryClient, {
      case: "deleted",
      value: create(moments.MomentRefSchema, { momentId: "a1", authorId: "aoi" }),
    });
    // Both were answered before the deletion.
    api.list(0, [moment("a1", "aoi"), moment("r1", "ren")]);
    api.map(0, [moment("a1", "aoi")]);
    await Promise.all([list, map]);
    expect(friendIds(queryClient)).toEqual(["r1"]);
    expect(mapIds(queryClient)).toEqual([]);
  });

  it("keeps a forgotten person out of the next page, even moments that were not cached yet", async () => {
    const queryClient = new QueryClient();
    const api = server();
    const observer = new InfiniteQueryObserver(queryClient, momentQueries.friends(api.client));
    const unsubscribe = observer.subscribe(() => {});
    api.list(0, [moment("r1", "ren")], "next");
    await observer.refetch();
    const next = observer.fetchNextPage();
    forgetUserMoments(queryClient, "aoi");
    api.list(1, [moment("a1", "aoi"), moment("r0", "ren")]);
    await next;
    expect(friendIds(queryClient)).toEqual(["r1", "r0"]);
    unsubscribe();
  });
});
