import { QueryClient } from "@tanstack/query-core";
import { chat, create } from "@tokuly/api";
import { describe, expect, it, vi } from "vitest";
import { chatKeys } from "./keys";
import { readTimeline } from "./message-cache";
import { type FetchPage, MessagePager, type Position } from "./pager";

const message = (seq: number) =>
  create(chat.MessageSchema, {
    id: `m${seq}`,
    conversationId: "c",
    seq: BigInt(seq),
    kind: chat.MessageKind.TEXT,
  });

/** A conversation of messages 1…100, served 10 at a time like the server pages them. */
const ALL = Array.from({ length: 100 }, (_, index) => message(index + 1));

function page(from: number, to: number): chat.ListMessagesResponse {
  const messages = ALL.slice(Math.max(0, from - 1), Math.min(100, to));
  return create(chat.ListMessagesResponseSchema, {
    messages,
    olderCursor: `before:${from}`,
    hasOlder: from > 1,
    newerCursor: `after:${to}`,
    hasNewer: to < 100,
  });
}

function serve(position: Position): chat.ListMessagesResponse {
  switch (position.case) {
    case "before": {
      const from = Number(position.value.split(":")[1]);
      return page(from - 10, from - 1);
    }
    case "after": {
      const to = Number(position.value.split(":")[1]);
      return page(to + 1, to + 10);
    }
    case "aroundMessageId": {
      const seq = Number(position.value.slice(1));
      if (seq > 100) throw new Error("MESSAGE_NOT_FOUND");
      return page(seq - 4, seq + 5);
    }
    case "aroundUnread":
      return page(41, 50);
    default:
      return page(91, 100);
  }
}

function pager() {
  const fetchPage = vi.fn<FetchPage>(async (_id, position) => serve(position));
  return { fetchPage, pager: new MessagePager(fetchPage) };
}

const seqs = (queryClient: QueryClient) =>
  readTimeline(queryClient, "c")?.messages.map((item) => Number(item.seq));

async function opened(pager: MessagePager, around?: string): Promise<QueryClient> {
  const queryClient = new QueryClient();
  queryClient.setQueryData(chatKeys.timeline("c"), await pager.first("c", around));
  return queryClient;
}

describe("message pager", () => {
  it("opens on the first unread message, or on the message asked for", async () => {
    const { pager: unread } = pager();
    expect(seqs(await opened(unread))).toEqual([41, 42, 43, 44, 45, 46, 47, 48, 49, 50]);
    const { pager: around } = pager();
    expect(seqs(await opened(around, "m20"))?.[0]).toBe(16);
  });

  it("opens normally when the message asked for is gone", async () => {
    const { pager: gone, fetchPage } = pager();
    expect(seqs(await opened(gone, "m500"))?.[0]).toBe(41);
    expect(fetchPage).toHaveBeenCalledTimes(2);
  });

  it("grows the window in both directions without gaps or doubles", async () => {
    const { pager: both } = pager();
    const queryClient = await opened(both);
    await both.loadOlder(queryClient, "c");
    await both.loadNewer(queryClient, "c");
    expect(seqs(queryClient)).toEqual(Array.from({ length: 30 }, (_, index) => 31 + index));
    expect(readTimeline(queryClient, "c")).toMatchObject({ hasOlder: true, hasNewer: true });
  });

  it("stops at the oldest message", async () => {
    const { pager: start, fetchPage } = pager();
    const queryClient = await opened(start, "m3");
    await start.loadOlder(queryClient, "c");
    expect(readTimeline(queryClient, "c")?.hasOlder).toBe(false);
    const calls = fetchPage.mock.calls.length;
    await start.loadOlder(queryClient, "c");
    expect(fetchPage).toHaveBeenCalledTimes(calls);
  });

  it("asks for the same page once while it is on its way", async () => {
    const { pager: busy, fetchPage } = pager();
    const queryClient = await opened(busy);
    await Promise.all([busy.loadOlder(queryClient, "c"), busy.loadOlder(queryClient, "c")]);
    expect(fetchPage).toHaveBeenCalledTimes(2);
  });

  it("catches up by starting over at the newest when too much was missed", async () => {
    const { pager: behind } = pager();
    const queryClient = await opened(behind);
    // At the live end as far as the client knows; the server has moved on by 50 messages.
    queryClient.setQueryData(chatKeys.timeline("c"), {
      ...readTimeline(queryClient, "c")!,
      hasNewer: false,
    });
    await behind.catchUp(queryClient, "c");
    expect(seqs(queryClient)?.at(-1)).toBe(100);
    expect(seqs(queryClient)?.[0]).toBe(91);
  });

  it("jumps to a message that is not held", async () => {
    const { pager: jump } = pager();
    const queryClient = await opened(jump);
    await jump.showMessage(queryClient, "c", "m80");
    expect(seqs(queryClient)?.[0]).toBe(76);
  });
});
