import { InfiniteQueryObserver, QueryClient } from "@tanstack/react-query";
import { common, create, engagement } from "@tokuly/api";
import { describe, expect, it, vi } from "vitest";
import type { ReplyPages, ThreadPages } from "./cache";

/** The comments server, answering when the test says so. */
const server = {
  lists: [] as ((comments: engagement.Comment[]) => void)[],
  replies: [] as ((replies: engagement.Comment[]) => void)[],
  comments: [] as ((comment: engagement.Comment | undefined) => void)[],
};
vi.mock("#/lib/api", () => ({
  api: () => ({
    service: () => ({
      listComments: () =>
        new Promise((resolve) =>
          server.lists.push((comments) =>
            resolve(create(engagement.ListCommentsResponseSchema, { comments })),
          ),
        ),
      listReplies: () =>
        new Promise((resolve) =>
          server.replies.push((replies) =>
            resolve(create(engagement.ListRepliesResponseSchema, { replies })),
          ),
        ),
      getComment: () =>
        new Promise((resolve) => server.comments.push((comment) => resolve({ comment }))),
    }),
  }),
}));
vi.mock("#/lib/realtime", () => ({ realtime: () => ({}) }));

const { commentClient, commentKeys } = await import("./api");
const { applyThreadEvent } = await import("./realtime");
const { journal } = await import("@tokuly/core/query");

const subject = { type: common.SubjectType.NOTE, id: "n1" };
const threadKey = commentKeys.thread(subject, engagement.CommentSort.TOP);
const comment = (id: string, parentId = "") =>
  create(engagement.CommentSchema, { id, parentId, text: id });

/** The thread as the screen reads it, on screen, with `comments` held from before. */
function threadOnScreen(queryClient: QueryClient, comments: engagement.Comment[]) {
  queryClient.setQueryData<ThreadPages>(threadKey, {
    pages: [create(engagement.ListCommentsResponseSchema, { comments })],
    pageParams: [""],
  });
  const observer = new InfiniteQueryObserver(queryClient, {
    queryKey: threadKey,
    queryFn: ({ signal }: { signal: AbortSignal }) =>
      commentClient().listComments({ subject, page: { limit: 20 } }, { signal }),
    persister: journal,
    initialPageParam: "",
    getNextPageParam: () => undefined,
    staleTime: 0,
  } as never);
  return observer.subscribe(() => {});
}
const listedIds = (queryClient: QueryClient) =>
  queryClient
    .getQueryData<ThreadPages>(threadKey)
    ?.pages.flatMap((page) => page.comments.map((item) => item.id));
const settled = () => new Promise((resolve) => setTimeout(resolve, 0));

describe("a comment removed while the thread is read again", () => {
  it("stays removed when the read answers with it", async () => {
    server.lists.length = 0;
    const queryClient = new QueryClient();
    const unwatch = threadOnScreen(queryClient, [comment("a"), comment("doomed")]);
    await vi.waitFor(() => expect(server.lists).toHaveLength(1));

    await applyThreadEvent(queryClient, subject, false, {
      case: "commentRemoved",
      value: create(engagement.CommentRemovedSchema, { commentId: "doomed", removedCount: 1 }),
    });
    expect(listedIds(queryClient)).toEqual(["a"]);
    // That read was answered before the comment was hidden.
    server.lists[0]?.([comment("a"), comment("doomed")]);
    await vi.waitFor(() => expect(queryClient.isFetching()).toBe(0));
    expect(listedIds(queryClient)).toEqual(["a"]);
    unwatch();
  });
});

describe("a comment fetched after commentAdded", () => {
  it("does not come back when it was removed while it was fetched", async () => {
    server.comments.length = 0;
    const queryClient = new QueryClient();
    queryClient.setQueryData<ThreadPages>(threadKey, {
      pages: [create(engagement.ListCommentsResponseSchema, { comments: [comment("a")] })],
      pageParams: [""],
    });
    // Its author has blocks in place: the push carries no comment, the page fetches it.
    const adding = applyThreadEvent(queryClient, subject, false, {
      case: "commentAdded",
      value: create(engagement.CommentAddedSchema, { commentId: "late" }),
    });
    await vi.waitFor(() => expect(server.comments).toHaveLength(1));
    await applyThreadEvent(queryClient, subject, false, {
      case: "commentRemoved",
      value: create(engagement.CommentRemovedSchema, { commentId: "late", removedCount: 1 }),
    });
    server.comments[0]?.(comment("late"));
    await adding;
    expect(listedIds(queryClient)).toEqual(["a"]);
  });
});

describe("replies", () => {
  const repliesKey = commentKeys.replies(subject, "a");

  it("are read again with the thread, and a reply removed meanwhile stays removed", async () => {
    server.replies.length = 0;
    const queryClient = new QueryClient();
    queryClient.setQueryData<ReplyPages>(repliesKey, {
      pages: [create(engagement.ListRepliesResponseSchema, { replies: [comment("r1", "a")] })],
      pageParams: [""],
    });
    const observer = new InfiniteQueryObserver(queryClient, {
      queryKey: repliesKey,
      queryFn: ({ signal }: { signal: AbortSignal }) =>
        commentClient().listReplies({ commentId: "a", page: { limit: 20 } }, { signal }),
      persister: journal,
      initialPageParam: "",
      getNextPageParam: () => undefined,
      staleTime: Number.POSITIVE_INFINITY,
    } as never);
    const unwatch = observer.subscribe(() => {});
    await settled();
    expect(server.replies).toHaveLength(0);

    // The topic is back after a gap: everything under the subject is read again, replies too.
    const { reread } = await import("@tokuly/core/query");
    void reread(queryClient, [commentKeys.subject(subject), commentKeys.repliesOf(subject)]);
    await vi.waitFor(() => expect(server.replies).toHaveLength(1));
    await applyThreadEvent(queryClient, subject, false, {
      case: "commentRemoved",
      value: create(engagement.CommentRemovedSchema, { commentId: "r1", parentId: "a" }),
    });
    server.replies[0]?.([comment("r1", "a"), comment("r2", "a")]);
    await vi.waitFor(() => expect(queryClient.isFetching()).toBe(0));
    expect(
      queryClient
        .getQueryData<ReplyPages>(repliesKey)
        ?.pages.flatMap((page) => page.replies.map((reply) => reply.id)),
    ).toEqual(["r2"]);
    unwatch();
  });
});

describe("the comment a link pointed at", () => {
  it("is forgotten when it is removed, also from a read of it on its way", async () => {
    server.comments.length = 0;
    const queryClient = new QueryClient();
    const key = commentKeys.focused("doomed");
    const reading = queryClient.fetchQuery({
      queryKey: key,
      queryFn: ({ signal }) => commentClient().getComment({ commentId: "doomed" }, { signal }),
      persister: journal,
    });
    await vi.waitFor(() => expect(server.comments).toHaveLength(1));
    await applyThreadEvent(queryClient, subject, false, {
      case: "commentRemoved",
      value: create(engagement.CommentRemovedSchema, { commentId: "doomed" }),
    });
    server.comments[0]?.(comment("doomed"));
    await reading;
    expect(queryClient.getQueryData(key)).toBeNull();
  });
});
