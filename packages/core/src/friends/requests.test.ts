import { create, social } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import { testI18n } from "../testing/i18n";
import { mutualFriendsOf, requestNote } from "./requests";

describe("mutualFriendsOf", () => {
  it("names the friends sent and counts the rest", () => {
    const request = create(social.FriendRequestSchema, {
      mutualFriendCount: 5,
      mutualFriends: [{ displayName: "みお" }, { handle: "sora" }],
    });
    expect(mutualFriendsOf(request)).toEqual({ names: ["みお", "@sora"], more: 3 });
  });

  it("is nothing without mutual friends", () => {
    expect(mutualFriendsOf(create(social.FriendRequestSchema))).toBeNull();
  });
});

describe("requestNote", () => {
  const i18n = testI18n("ja");

  it("puts the message and whom both know on one line", () => {
    const request = create(social.FriendRequestSchema, {
      message: "よろしく！",
      mutualFriendCount: 3,
      mutualFriends: [{ displayName: "みお" }, { displayName: "そら" }],
    });
    expect(requestNote(i18n, request)).toBe("よろしく！ · みお、そらほか1人と共通のともだち");
  });

  it("counts when no names came, and is nothing without either", () => {
    expect(requestNote(i18n, create(social.FriendRequestSchema, { mutualFriendCount: 2 }))).toBe(
      "共通のともだち 2人",
    );
    expect(requestNote(i18n, create(social.FriendRequestSchema))).toBeUndefined();
  });
});
