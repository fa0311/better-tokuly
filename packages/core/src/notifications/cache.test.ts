import { QueryClient } from "@tanstack/query-core";
import { create, notify } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import { applyInboxChange, storeUnread } from "./cache";
import { entriesOf, type InboxPages, withReceived } from "./inbox-cache";
import { notifyKeys } from "./keys";
import type { Unread } from "./unread";

const entry = (id: string, category = "social") =>
  create(notify.NotificationSchema, { id, category, title: id, groupCount: 1 });

const pages = (...entries: notify.Notification[]): InboxPages => ({
  pages: [create(notify.ListNotificationsResponseSchema, { notifications: entries })],
  pageParams: [""],
});

describe("inbox in the cache", () => {
  it("applies a change to every view by its own filter", () => {
    const queryClient = new QueryClient();
    const all = { category: "", unreadOnly: false };
    const chat = { category: "chat", unreadOnly: false };
    queryClient.setQueryData(notifyKeys.inboxList(all), pages(entry("a")));
    queryClient.setQueryData(notifyKeys.inboxList(chat), pages());
    applyInboxChange(queryClient, (held, filter) => withReceived(held, entry("b"), filter));
    const ids = (filter: typeof all) =>
      entriesOf(queryClient.getQueryData<InboxPages>(notifyKeys.inboxList(filter))).map(
        (item) => item.id,
      );
    expect(ids(all)).toEqual(["b", "a"]);
    expect(ids(chat)).toEqual([]);
  });

  it("stores the unread counts an answer carried, and ignores an answer without them", () => {
    const queryClient = new QueryClient();
    storeUnread(
      queryClient,
      create(notify.UnreadCountSchema, { total: 2, byCategory: { chat: 2 } }),
    );
    storeUnread(queryClient, undefined);
    expect(queryClient.getQueryData<Unread>(notifyKeys.unread)).toEqual({
      total: 2,
      byCategory: { chat: 2 },
    });
  });
});
