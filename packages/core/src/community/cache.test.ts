import { create } from "@bufbuild/protobuf";
import { QueryClient } from "@tanstack/query-core";
import { community } from "@tokuly/api";
import { describe, expect, it, vi } from "vitest";
import { applyCommunityEvent, rereadCommunity, upsertReply } from "./cache";
import { communityKeys } from "./keys";
import { communityQueries } from "./queries";

const thread = create(community.ThreadSchema, { id: "t1", communityId: "c1", replyCount: 1 });
const reply = (id: string) => create(community.ReplySchema, { id, threadId: "t1" });
const noBoards = () => {
  throw new Error("not asked");
};

type Pages<T> = { pages: { items: T[]; nextCursor: string }[] };

describe("threads and replies in the cache", () => {
  it("a new reply goes on top of a newest-first list and at the end of a complete oldest-first one", () => {
    const queryClient = new QueryClient();
    const page = { items: [reply("r1")], nextCursor: "" };
    queryClient.setQueryData(communityKeys.replies("t1", true), {
      pages: [page],
      pageParams: [""],
    });
    queryClient.setQueryData(communityKeys.replies("t1", false), {
      pages: [page],
      pageParams: [""],
    });
    upsertReply(queryClient, reply("r2"));
    const ids = (newestFirst: boolean) =>
      queryClient
        .getQueryData<Pages<community.Reply>>(communityKeys.replies("t1", newestFirst))
        ?.pages[0]?.items.map((each) => each.id);
    expect(ids(true)).toEqual(["r2", "r1"]);
    expect(ids(false)).toEqual(["r1", "r2"]);
  });

  it("a removed reply leaves the list and the thread's count follows, on its page and in lists", () => {
    const queryClient = new QueryClient();
    const options = { channelId: "", sort: 0, window: 0, tag: "" };
    queryClient.setQueryData(communityKeys.thread("t1"), thread);
    queryClient.setQueryData(communityKeys.threads("c1", options), {
      pages: [{ pinned: [], items: [thread], nextCursor: "" }],
      pageParams: [""],
    });
    queryClient.setQueryData(communityKeys.replies("t1", false), {
      pages: [{ items: [reply("r1")], nextCursor: "" }],
      pageParams: [""],
    });
    applyCommunityEvent(
      queryClient,
      {
        case: "replyRemoved",
        value: create(community.ReplyRemovedSchema, {
          communityId: "c1",
          threadId: "t1",
          replyId: "r1",
          replyCount: 0,
        }),
      },
      noBoards,
    );
    expect(queryClient.getQueryData<community.Thread>(communityKeys.thread("t1"))?.replyCount).toBe(
      0,
    );
    const list = queryClient.getQueryData<Pages<community.Thread>>(
      communityKeys.threads("c1", options),
    );
    expect(list?.pages[0]?.items[0]?.replyCount).toBe(0);
    const replies = queryClient.getQueryData<Pages<community.Reply>>(
      communityKeys.replies("t1", false),
    );
    expect(replies?.pages[0]?.items).toEqual([]);
  });
});

describe("reads on their way", () => {
  /** A board server whose answers the test hands out. */
  function slowBoards() {
    const replies: ((items: community.Reply[]) => void)[] = [];
    const got: ((reply: community.Reply) => void)[] = [];
    const threads: ((thread: community.Thread) => void)[] = [];
    const signals: AbortSignal[] = [];
    const client = {
      listReplies: (_request: unknown, options: { signal: AbortSignal }) => {
        signals.push(options.signal);
        return new Promise((resolve) =>
          replies.push((items) => resolve({ replies: items, page: { nextCursor: "" } })),
        );
      },
      getThread: () => new Promise((resolve) => threads.push((thread) => resolve({ thread }))),
      getReply: () => new Promise((resolve) => got.push((reply) => resolve({ reply }))),
    };
    return { boards: () => client as never, replies, got, threads, signals };
  }
  const removed = (replyId: string) =>
    ({
      case: "replyRemoved",
      value: create(community.ReplyRemovedSchema, {
        communityId: "c1",
        threadId: "t1",
        replyId,
        replyCount: 1,
      }),
    }) as const;
  const ids = (queryClient: QueryClient) =>
    queryClient
      .getQueryData<Pages<community.Reply>>(communityKeys.replies("t1", false))
      ?.pages.flatMap((page) => page.items.map((each) => each.id));

  it("a reply removed while the replies are read stays removed", async () => {
    const { boards, replies } = slowBoards();
    const queryClient = new QueryClient();
    const reading = queryClient.fetchInfiniteQuery(communityQueries.replies(boards, "t1", false));
    await vi.waitFor(() => expect(replies).toHaveLength(1));
    applyCommunityEvent(queryClient, removed("doomed"), boards);
    // That page was read before the reply was removed.
    replies[0]?.([reply("r1"), reply("doomed")]);
    await reading;
    expect(ids(queryClient)).toEqual(["r1"]);
  });

  it("a reply fetched after replyCreated does not come back when it was removed meanwhile", async () => {
    const { boards, got } = slowBoards();
    const queryClient = new QueryClient();
    queryClient.setQueryData(communityKeys.replies("t1", false), {
      pages: [{ items: [reply("r1")], nextCursor: "" }],
      pageParams: [""],
    });
    applyCommunityEvent(
      queryClient,
      {
        case: "replyCreated",
        value: create(community.ReplyCreatedSchema, { threadId: "t1", replyId: "late" }),
      },
      boards,
    );
    await vi.waitFor(() => expect(got).toHaveLength(1));
    applyCommunityEvent(queryClient, removed("late"), boards);
    got[0]?.(reply("late"));
    await new Promise((resolve) => setTimeout(resolve));
    expect(ids(queryClient)).toEqual(["r1"]);
  });

  it("a thread's topic reads its replies again, replacing a read on its way", async () => {
    const { boards, replies, signals } = slowBoards();
    const queryClient = new QueryClient();
    queryClient.setQueryData(communityKeys.thread("t1-short"), thread);
    const reading = queryClient.fetchInfiniteQuery(communityQueries.replies(boards, "t1", false));
    await vi.waitFor(() => expect(replies).toHaveLength(1));
    void rereadCommunity(queryClient, "c1");
    expect(signals[0]?.aborted).toBe(true);
    await vi.waitFor(() => expect(replies).toHaveLength(2));
    replies[1]?.([reply("r2")]);
    await reading;
    expect(ids(queryClient)).toEqual(["r2"]);
  });

  it("counts pushed while a thread is first read under its short ID are kept", async () => {
    const { boards, threads } = slowBoards();
    const queryClient = new QueryClient();
    const reading = queryClient.fetchQuery(communityQueries.thread(boards, "t1-short"));
    await vi.waitFor(() => expect(threads).toHaveLength(1));
    applyCommunityEvent(
      queryClient,
      {
        case: "threadCountsChanged",
        value: create(community.ThreadCountsChangedSchema, {
          threadId: "t1",
          replyCount: 5,
          likeCount: 9n,
        }),
      },
      boards,
    );
    threads[0]?.(thread);
    expect((await reading)?.replyCount).toBe(5);
    expect(
      queryClient.getQueryData<community.Thread>(communityKeys.thread("t1-short"))?.likeCount,
    ).toBe(9n);
  });
});
