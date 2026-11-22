import { create, engagement } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import {
  compactCount,
  type Engagement,
  emojiReaction,
  LIKE,
  toEngagement,
  withCounters,
  withViewerReaction,
} from "./model";

const base: Engagement = {
  subjectId: "s1",
  likeCount: 2,
  dislikeCount: 0,
  emojiCounts: [
    { emoji: "🎉", count: 3 },
    { emoji: "🔥", count: 1 },
  ],
  emojiTotal: 4,
  commentCount: 0,
  bookmarkCount: 0,
  shareCount: 0,
  bookmarked: false,
  interactionsEnabled: true,
};

describe("withViewerReaction", () => {
  it("adds a like", () => {
    const next = withViewerReaction(base, LIKE);
    expect(next.likeCount).toBe(3);
    expect(next.viewerReaction).toEqual(LIKE);
  });

  it("joins an emoji that is already there and starts a new one", () => {
    expect(withViewerReaction(base, emojiReaction("🎉")).emojiCounts[0]).toEqual({
      emoji: "🎉",
      count: 4,
    });
    const fresh = withViewerReaction(base, emojiReaction("✨"));
    expect(fresh.emojiCounts.at(-1)).toEqual({ emoji: "✨", count: 1 });
    expect(fresh.emojiTotal).toBe(5);
  });

  it("moves the viewer from one reaction to another: one leaves, one joins", () => {
    const reacted = withViewerReaction(base, emojiReaction("🔥"));
    const moved = withViewerReaction(reacted, LIKE);
    expect(moved.emojiCounts).toEqual([
      { emoji: "🎉", count: 3 },
      { emoji: "🔥", count: 1 },
    ]);
    expect(moved.emojiTotal).toBe(4);
    expect(moved.likeCount).toBe(3);
  });

  it("drops an emoji nobody uses anymore when the reaction is taken back", () => {
    const only = withViewerReaction(
      { ...base, emojiCounts: [], emojiTotal: 0 },
      emojiReaction("✨"),
    );
    const back = withViewerReaction(only, undefined);
    expect(back.emojiCounts).toEqual([]);
    expect(back.emojiTotal).toBe(0);
    expect(back.viewerReaction).toBeUndefined();
  });
});

describe("server data", () => {
  it("reads an engagement message", () => {
    const message = create(engagement.SubjectEngagementSchema, {
      subject: { type: 3, id: "uuid" },
      reactions: {
        likeCount: 5n,
        emojiCounts: [{ emoji: "🎉", count: 2n }],
        emojiTotal: 2n,
        viewerReaction: { kind: engagement.ReactionKind.EMOJI, emoji: "🎉" },
      },
      bookmarkCount: 1n,
      bookmarked: true,
      interactionsEnabled: true,
    });
    expect(toEngagement(message)).toMatchObject({
      subjectId: "uuid",
      likeCount: 5,
      emojiCounts: [{ emoji: "🎉", count: 2 }],
      viewerReaction: { emoji: "🎉" },
      bookmarked: true,
    });
  });

  it("applies pushed counters without losing the viewer's own state", () => {
    const mine = withViewerReaction({ ...base, bookmarked: true }, emojiReaction("🎉"));
    const pushed = create(engagement.CountersChangedSchema, {
      reactions: { likeCount: 9n, emojiCounts: [{ emoji: "🎉", count: 7n }], emojiTotal: 7n },
      bookmarkCount: 4n,
      shareCount: 2n,
    });
    const next = withCounters(mine, pushed);
    expect(next).toMatchObject({
      likeCount: 9,
      emojiTotal: 7,
      bookmarkCount: 4,
      shareCount: 2,
      bookmarked: true,
    });
    expect(next.viewerReaction).toEqual(emojiReaction("🎉"));
  });
});

describe("compactCount", () => {
  it("keeps small counts exact and shortens big ones", () => {
    expect(compactCount(1234, "en")).toBe("1,234");
    expect(compactCount(12_345, "en")).toBe("12.3K");
    expect(compactCount(12_345, "ja")).toBe("1.2万");
  });
});
