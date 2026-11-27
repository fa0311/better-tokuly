import { create, social } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import { mutualFriendsOf } from "./requests";

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
