import { create } from "@bufbuild/protobuf";
import { QueryClient } from "@tanstack/query-core";
import { community } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import { applyCommunityEvent, upsertReply } from "./cache";
import { communityKeys } from "./keys";

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
