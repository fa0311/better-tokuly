import { create, notify } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import { entriesOf, type InboxFilter, type InboxPages, withRead, withReceived, withRemoved } from "./inbox-cache";

const ALL: InboxFilter = { category: "", unreadOnly: false };
const UNREAD: InboxFilter = { category: "", unreadOnly: true };
const CHAT: InboxFilter = { category: "chat", unreadOnly: false };

function entry(id: string, category = "social", read = false): notify.Notification {
  return create(notify.NotificationSchema, { id, category, kind: `${category}.test`, title: id, read, groupCount: 1 });
}

/** Two pages, as an infinite query holds them. */
function pages(...lists: notify.Notification[][]): InboxPages {
  return {
    pages: lists.map((notifications) => create(notify.ListNotificationsResponseSchema, { notifications })),
    pageParams: lists.map((_, index) => (index === 0 ? "" : `cursor-${index}`)),
  };
}

const ids = (data: InboxPages) => entriesOf(data).map((item) => item.id);

describe("inbox cache", () => {
  it("puts a new notification on top of the first page", () => {
    const data = withReceived(pages([entry("b"), entry("a")], [entry("z")]), entry("c"), ALL);
    expect(ids(data)).toEqual(["c", "b", "a", "z"]);
    expect(data.pages[0]?.notifications).toHaveLength(3);
  });

  it("moves a grouped entry that was replaced to the top instead of duplicating it", () => {
    const grown = create(notify.NotificationSchema, { ...entry("z"), groupCount: 4 });
    const data = withReceived(pages([entry("b"), entry("a")], [entry("z")]), grown, ALL);
    expect(ids(data)).toEqual(["z", "b", "a"]);
    expect(entriesOf(data)[0]?.groupCount).toBe(4);
  });

  it("only shows a new notification in the views whose filter it matches", () => {
    const start = pages([entry("a", "chat")]);
    expect(ids(withReceived(start, entry("b", "social"), CHAT))).toEqual(["a"]);
    expect(ids(withReceived(start, entry("b", "chat"), CHAT))).toEqual(["b", "a"]);
    expect(ids(withReceived(start, entry("b", "chat", true), UNREAD))).toEqual(["a"]);
  });

  it("marks entries read, and drops them from a view of unread entries", () => {
    const start = pages([entry("b"), entry("a")]);
    const read = withRead(start, new Set(["a"]), ALL);
    expect(entriesOf(read).map((item) => item.read)).toEqual([false, true]);
    expect(ids(withRead(start, new Set(["a"]), UNREAD))).toEqual(["b"]);
  });

  it("marks everything read, or everything of one category", () => {
    const start = pages([entry("c", "chat"), entry("s", "social")]);
    expect(entriesOf(withRead(start, "all", ALL)).every((item) => item.read)).toBe(true);
    expect(entriesOf(withRead(start, "all", ALL, "chat")).map((item) => item.read)).toEqual([true, false]);
    expect(ids(withRead(start, "all", UNREAD, "chat"))).toEqual(["s"]);
  });

  it("removes deleted entries from every page", () => {
    const data = withRemoved(pages([entry("c"), entry("b")], [entry("a")]), new Set(["c", "a"]));
    expect(ids(data)).toEqual(["b"]);
    expect(data.pages).toHaveLength(2);
  });

  it("clears a category, or only what was read", () => {
    const start = pages([entry("c", "chat", true), entry("d", "chat"), entry("s", "social", true)]);
    expect(ids(withRemoved(start, { category: "", readOnly: false }))).toEqual([]);
    expect(ids(withRemoved(start, { category: "chat", readOnly: false }))).toEqual(["s"]);
    expect(ids(withRemoved(start, { category: "", readOnly: true }))).toEqual(["d"]);
    expect(ids(withRemoved(start, { category: "chat", readOnly: true }))).toEqual(["d", "s"]);
  });
});
