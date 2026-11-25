import { create, notify } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import { NOTHING_UNREAD, unreadOf } from "./unread";

describe("unreadOf", () => {
  it("copies the counts the server sent, and nothing when it sent none", () => {
    const sent = create(notify.UnreadCountSchema, { total: 3, byCategory: { social: 2 } });
    const unread = unreadOf(sent);
    expect(unread).toEqual({ total: 3, byCategory: { social: 2 } });
    expect(unread.byCategory).not.toBe(sent.byCategory);
    expect(unreadOf(undefined)).toBe(NOTHING_UNREAD);
  });
});
