import { QueryClient } from "@tanstack/query-core";
import { chat, create, fromDate } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import {
  dropConversation,
  patchConversation,
  putConversation,
  readConversation,
} from "./conversation-cache";
import { chatKeys } from "./keys";
import { readTimeline, storeCreated, storeOwnRead } from "./message-cache";
import { timelineOf } from "./timeline";
import { adjustUnread, NO_UNREAD, type Unread } from "./unread";

const conversation = (id: string, lastSeq = 1n, unreadCount = 0) =>
  create(chat.ConversationSchema, {
    id,
    kind: chat.ConversationKind.GROUP,
    lastSeq,
    lastActivityAt: fromDate(new Date(2026, 10, 1)),
    viewer: { requestState: chat.RequestState.ACCEPTED, unreadCount, readSeq: lastSeq - 1n },
  });

const message = (id: string, seq: bigint, conversationId = "c") =>
  create(chat.MessageSchema, {
    id,
    seq,
    conversationId,
    kind: chat.MessageKind.TEXT,
    sender: { id: "aoi", handle: "aoi" },
    createdAt: fromDate(new Date(2026, 10, 2, 0, Number(seq))),
  });

/** A client holding conversation `c` in the inbox, opened with messages 1 and 2. */
function holding(): QueryClient {
  const queryClient = new QueryClient();
  queryClient.setQueryData(chatKeys.list("inbox"), {
    pages: [{ conversations: [conversation("c", 2n)], nextCursor: "" }],
    pageParams: [""],
  });
  queryClient.setQueryData(chatKeys.conversation("c"), conversation("c", 2n));
  queryClient.setQueryData(
    chatKeys.timeline("c"),
    timelineOf({
      messages: [message("m1", 1n), message("m2", 2n)],
      olderCursor: "",
      hasOlder: false,
      newerCursor: "",
      hasNewer: false,
    }),
  );
  return queryClient;
}

const inbox = (queryClient: QueryClient) =>
  queryClient
    .getQueryData<{ pages: { conversations: chat.Conversation[] }[] }>(chatKeys.list("inbox"))
    ?.pages.flatMap((page) => page.conversations) ?? [];

describe("conversation cache", () => {
  it("keeps the open conversation and its list row in step", () => {
    const queryClient = holding();
    expect(patchConversation(queryClient, "c", (held) => ({ ...held, name: "遊び" }))).toBe(true);
    expect(readConversation(queryClient, "c")?.name).toBe("遊び");
    expect(inbox(queryClient)[0]?.name).toBe("遊び");
    expect(patchConversation(queryClient, "unknown", (held) => held)).toBe(false);
  });

  it("does not start holding a conversation nobody opened", () => {
    const queryClient = holding();
    putConversation(queryClient, conversation("new", 5n));
    expect(queryClient.getQueryData(chatKeys.conversation("new"))).toBeUndefined();
    expect(inbox(queryClient).map((item) => item.id)).toContain("new");
  });

  it("forgets a dropped conversation everywhere", () => {
    const queryClient = holding();
    dropConversation(queryClient, "c");
    expect(inbox(queryClient)).toEqual([]);
    expect(readConversation(queryClient, "c")).toBeUndefined();
    expect(readTimeline(queryClient, "c")).toBeUndefined();
  });
});

describe("message cache", () => {
  it("adds the next message to the window and the preview", () => {
    const queryClient = holding();
    expect(storeCreated(queryClient, message("m3", 3n))).toEqual({ known: true, missed: false });
    expect(readTimeline(queryClient, "c")?.messages.map((item) => item.id)).toEqual([
      "m1",
      "m2",
      "m3",
    ]);
    expect(readConversation(queryClient, "c")?.lastMessage?.id).toBe("m3");
  });

  it("leaves a hole for the caller to fetch instead of filling it", () => {
    const queryClient = holding();
    expect(storeCreated(queryClient, message("m9", 9n))).toEqual({ known: true, missed: true });
    expect(readTimeline(queryClient, "c")?.messages).toHaveLength(2);
    expect(storeCreated(queryClient, message("x1", 1n, "other")).known).toBe(false);
  });

  it("moves the viewer's read state forward only", () => {
    const queryClient = holding();
    storeOwnRead(queryClient, "c", 2n, 0);
    storeOwnRead(queryClient, "c", 1n, 0);
    expect(readConversation(queryClient, "c")?.viewer?.readSeq).toBe(2n);
  });
});

describe("unread totals", () => {
  it("adjust the held totals, never below zero, and say when none are held", () => {
    const queryClient = new QueryClient();
    expect(adjustUnread(queryClient, (unread) => unread)).toBe(false);
    queryClient.setQueryData<Unread>(chatKeys.unread, { ...NO_UNREAD, badge: 1 });
    expect(adjustUnread(queryClient, (unread) => ({ ...unread, badge: unread.badge - 2 }))).toBe(
      true,
    );
    expect(queryClient.getQueryData<Unread>(chatKeys.unread)?.badge).toBe(0);
  });
});
