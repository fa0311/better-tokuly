import { create, engagement } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import {
  compactCount,
  DEFAULT_RULES,
  type Engagement,
  emojiReaction,
  LIKE,
  toEngagement,
  withCounters,
  withReactionChange,
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
  distinctEmoji: 2,
  viewerReactions: [],
  rules: DEFAULT_RULES,
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

describe("withReactionChange", () => {
  const several: Engagement = {
    ...base,
    rules: { ...DEFAULT_RULES, multiple: true, maxPerPerson: 5 },
  };

  it("puts emoji side by side where a subject takes several", () => {
    const one = withReactionChange(several, { add: emojiReaction("🎉") });
    const two = withReactionChange(one, { add: emojiReaction(":neko:") });
    expect(two.viewerReactions.map((reaction) => reaction.emoji)).toEqual(["🎉", ":neko:"]);
    expect(two.emojiTotal).toBe(6);
    expect(two.emojiCounts.at(-1)).toEqual({ emoji: ":neko:", count: 1 });
    expect(two.viewerReaction?.emoji).toBe(":neko:");
    // A like joins them, and stands for them.
    const liked = withReactionChange(two, { add: LIKE });
    expect(liked.viewerReactions).toHaveLength(3);
    expect(liked.viewerReaction).toEqual(LIKE);
  });

  it("takes back one emoji, or everything", () => {
    const two = withReactionChange(withReactionChange(several, { add: emojiReaction("🎉") }), {
      add: emojiReaction("🔥"),
    });
    const one = withReactionChange(two, { remove: emojiReaction("🎉") });
    expect(one.viewerReactions.map((reaction) => reaction.emoji)).toEqual(["🔥"]);
    expect(one.emojiCounts[0]).toEqual({ emoji: "🎉", count: 3 });
    const none = withReactionChange(two, { removeAll: true });
    expect(none.viewerReactions).toEqual([]);
    expect(none.emojiTotal).toBe(4);
  });

  it("replaces the reaction where a subject takes one", () => {
    const first = withReactionChange(base, { add: emojiReaction("🎉") });
    const second = withReactionChange(first, { add: emojiReaction("🔥") });
    expect(second.viewerReactions.map((reaction) => reaction.emoji)).toEqual(["🔥"]);
    expect(second.emojiTotal).toBe(5);
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

  it("reads custom emoji with their pictures, rules and every reaction of the viewer", () => {
    const neko = {
      name: "neko",
      image: {
        url: "https://m/anim.webp",
        variants: [
          { size: 128, url: "https://m/128.webp" },
          { size: 64, url: "https://m/64.webp" },
        ],
      },
      staticUrl: "https://m/64.webp",
      animated: true,
      available: true,
    };
    const message = create(engagement.SubjectEngagementSchema, {
      subject: { type: 22, id: "uuid" },
      reactions: {
        emojiCounts: [
          { emoji: ":neko:", count: 1n, custom: neko },
          { emoji: ":gone:", count: 2n, custom: { name: "gone" } },
        ],
        emojiTotal: 3n,
        viewerReaction: { kind: engagement.ReactionKind.EMOJI, emoji: ":neko:", custom: neko },
        viewerReactions: [
          { kind: engagement.ReactionKind.EMOJI, emoji: "✅" },
          { kind: engagement.ReactionKind.EMOJI, emoji: ":neko:", custom: neko },
        ],
        rules: {
          emoji: true,
          multiple: true,
          maxPerPerson: 20,
          maxDistinctEmoji: 50,
          customEmoji: true,
        },
      },
    });
    const read = toEngagement(message);
    expect(read.emojiCounts[0]?.custom).toMatchObject({
      name: "neko",
      animated: true,
      available: true,
      staticUrl: "https://m/64.webp",
      stills: [
        { size: 64, url: "https://m/64.webp" },
        { size: 128, url: "https://m/128.webp" },
      ],
    });
    expect(read.emojiCounts[1]?.custom?.available).toBe(false);
    expect(read.viewerReactions.map((reaction) => reaction.emoji)).toEqual(["✅", ":neko:"]);
    expect(read.rules).toMatchObject({ multiple: true, maxPerPerson: 20, sensitiveEmoji: false });
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
