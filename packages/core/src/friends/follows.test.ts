import { QueryClient } from "@tanstack/query-core";
import { create, identity, social } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import { profileKeys } from "../profile/keys";
import { FOLLOW_CARD, refreshFollows, summaryFromProfile } from "./follows";
import { followKeys } from "./keys";

describe("summaryFromProfile", () => {
  it("reads the follow card a profile carries", () => {
    const profile = create(identity.ProfileSchema, {
      user: { id: "aoi", handle: "aoi" },
      cards: [
        {
          kind: FOLLOW_CARD,
          data: {
            follower_count: "12",
            following_count: "3",
            locked: "true",
            following: "REQUESTED",
            follows_you: "true",
          },
        },
      ],
    });
    const summary = summaryFromProfile(profile);
    expect(summary?.followerCount).toBe(12);
    expect(summary?.locked).toBe(true);
    expect(summary?.relation?.following).toBe(social.FollowState.REQUESTED);
    expect(summary?.relation?.followsYou).toBe(true);
  });

  it("has nothing to say without the card", () => {
    expect(summaryFromProfile(create(identity.ProfileSchema, { user: { id: "aoi" } }))).toBe(
      undefined,
    );
  });
});

describe("refreshFollows", () => {
  it("re-reads every follow list and every profile", async () => {
    const queryClient = new QueryClient();
    queryClient.setQueryData(followKeys.summary("aoi"), 1);
    queryClient.setQueryData(profileKeys.profile("aoi"), 1);
    await refreshFollows(queryClient);
    expect(queryClient.getQueryState(followKeys.summary("aoi"))?.isInvalidated).toBe(true);
    expect(queryClient.getQueryState(profileKeys.profile("aoi"))?.isInvalidated).toBe(true);
  });
});
