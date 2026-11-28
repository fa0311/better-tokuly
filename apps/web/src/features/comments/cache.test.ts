import { create, engagement, type MessageInitShape } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import {
  findComment,
  mergeComment,
  type ReplyPages,
  type ThreadPages,
  withComment,
  withoutComment,
  withoutReply,
  withPatched,
  withPinned,
  withReply,
  withReplyCountMoved,
  withTotal,
  withUpdated,
} from "./cache";

const comment = (id: string, fields: MessageInitShape<typeof engagement.CommentSchema> = {}) =>
  create(engagement.CommentSchema, { text: id, ...fields, id });
const viewer = create(engagement.CommentViewerStateSchema, {
  isAuthor: true,
  canEdit: true,
  canDelete: true,
});

function thread(pages: engagement.Comment[][], total = 0n): ThreadPages {
  return {
    pageParams: pages.map((_, index) => String(index)),
    pages: pages.map((comments, index) =>
      create(engagement.ListCommentsResponseSchema, {
        comments,
        totalCount: total,
        page: { nextCursor: index < pages.length - 1 ? "more" : "" },
      }),
    ),
  };
}
function replies(pages: engagement.Comment[][], more = false): ReplyPages {
  return {
    pageParams: pages.map((_, index) => String(index)),
    pages: pages.map((items, index) =>
      create(engagement.ListRepliesResponseSchema, {
        replies: items,
        page: { nextCursor: more || index < pages.length - 1 ? "more" : "" },
      }),
    ),
  };
}
const ids = (data: ThreadPages) => data.pages.map((page) => page.comments.map((item) => item.id));

describe("comment thread cache", () => {
  it("adds a new comment at the top, under the pinned one, and counts it", () => {
    const data = thread([[comment("pin", { pinned: true }), comment("a")], [comment("b")]], 3n);
    const next = withComment(data, comment("new"), true);
    expect(ids(next)).toEqual([["pin", "new", "a"], ["b"]]);
    expect(next.pages.map((page) => page.totalCount)).toEqual([4n, 4n]);
  });

  it("does not count a comment that is held for review", () => {
    const next = withComment(thread([[comment("a")]], 1n), comment("held"), false);
    expect(next.pages[0]?.totalCount).toBe(1n);
    expect(ids(next)).toEqual([["held", "a"]]);
  });

  it("replaces a comment that is already listed instead of adding it twice", () => {
    const data = thread([[comment("a", { viewer })]], 1n);
    const next = withComment(data, comment("a", { text: "edited" }), true);
    expect(ids(next)).toEqual([["a"]]);
    expect(next.pages[0]?.totalCount).toBe(1n);
    expect(findComment(next, "a")?.text).toBe("edited");
  });

  it("keeps the viewer state when a pushed comment has none", () => {
    expect(mergeComment(comment("a", { viewer }), comment("a", { hearted: true })).viewer).toBe(
      viewer,
    );
    const fresh = create(engagement.CommentViewerStateSchema, { canModerate: true });
    expect(mergeComment(comment("a", { viewer }), comment("a", { viewer: fresh })).viewer).toBe(
      fresh,
    );
  });

  it("ignores updates of comments that are not loaded", () => {
    const data = thread([[comment("a")]]);
    expect(withUpdated(data, comment("zzz"))).toBe(data);
    expect(withPatched(data, "zzz", (item) => item)).toBe(data);
  });

  it("removes a comment and takes its replies off the count", () => {
    const next = withoutComment(thread([[comment("a"), comment("b")]], 5n), "a", 3);
    expect(ids(next)).toEqual([["b"]]);
    expect(next.pages[0]?.totalCount).toBe(2n);
    expect(withoutComment(thread([[comment("a")]], 1n), "a", 4).pages[0]?.totalCount).toBe(0n);
    // Applied again, or to a list read after the removal: nothing more comes off.
    expect(withoutComment(next, "a", 3).pages[0]?.totalCount).toBe(2n);
  });

  it("moves a reply count once, however often the change is applied", () => {
    const data = thread([[comment("a", { replyCount: 2 })]]);
    const count = (next: ThreadPages) => findComment(next, "a")?.replyCount;
    const added = withReplyCountMoved(data, "a", 2, 1);
    expect(count(added)).toBe(3);
    expect(count(withReplyCountMoved(added, "a", 2, 1))).toBe(3);
    const removed = withReplyCountMoved(added, "a", 3, -1);
    expect(count(removed)).toBe(2);
    expect(count(withReplyCountMoved(removed, "a", 3, -1))).toBe(2);
  });

  it("moves the pin: one pinned comment, leading the first page", () => {
    const data = thread([[comment("old", { pinned: true }), comment("a")], [comment("b")]]);
    const next = withPinned(data, "b", true);
    expect(ids(next)).toEqual([["b", "old", "a"], []]);
    expect(findComment(next, "b")?.pinned).toBe(true);
    expect(findComment(next, "old")?.pinned).toBe(false);
    const cleared = withPinned(next, "b", false);
    expect(cleared.pages.flatMap((page) => page.comments).some((item) => item.pinned)).toBe(false);
  });

  it("sets the count the server reports", () => {
    expect(withTotal(thread([[comment("a")]], 1n), 9n).pages[0]?.totalCount).toBe(9n);
  });
});

describe("reply cache", () => {
  it("appends a new reply once every page is loaded", () => {
    const next = withReply(replies([[comment("r1")]]), comment("r2"));
    expect(next.pages[0]?.replies.map((reply) => reply.id)).toEqual(["r1", "r2"]);
  });

  it("leaves a pushed reply for its page while older pages are still to come", () => {
    const data = replies([[comment("r1")]], true);
    expect(withReply(data, comment("r9"))).toBe(data);
    // The viewer's own reply is shown at once all the same.
    expect(
      withReply(data, comment("mine"), true).pages[0]?.replies.map((reply) => reply.id),
    ).toEqual(["r1", "mine"]);
  });

  it("updates a reply in place and removes one", () => {
    const data = replies([[comment("r1"), comment("r2")]]);
    expect(withReply(data, comment("r1", { text: "edited" })).pages[0]?.replies[0]?.text).toBe(
      "edited",
    );
    expect(withoutReply(data, "r1").pages[0]?.replies.map((reply) => reply.id)).toEqual(["r2"]);
    expect(withoutReply(data, "zzz")).toBe(data);
  });
});
