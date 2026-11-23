import { create } from "@bufbuild/protobuf";
import { live } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import {
  activeHighlights,
  addChatMessages,
  clampClipRange,
  countdownParts,
  fillAlertTemplate,
  giftAttempt,
  maskSecret,
  maskUrl,
  mergePoll,
  newIdempotencyKey,
  parseCoins,
  pollShares,
  removeChatMessage,
  removeChatUser,
  sameGift,
  slowModeWait,
  tierFor,
  tierStyle,
} from "./model";

const tier = (level: number, minCoins: bigint, gradientColors: string[] = []) =>
  create(live.GiftTierSchema, { level, minCoins, backgroundColor: "#111111", gradientColors });

const TIERS = [tier(1, 10n), tier(2, 50n), tier(3, 100n), tier(12, 50_000n)];

const line = (id: string, offsetMs: number, userId = "u1", coins?: bigint, untilSeconds = 0) =>
  create(live.ChatMessageSchema, {
    id,
    offsetMs: BigInt(offsetMs),
    author: { user: { id: userId } },
    gift:
      coins === undefined
        ? undefined
        : { coins, highlightedUntil: { seconds: BigInt(untilSeconds), nanos: 0 } },
  });

describe("gift tiers", () => {
  it("puts a gift in the highest tier it reaches", () => {
    expect(tierFor(TIERS, 10)?.level).toBe(1);
    expect(tierFor(TIERS, 99n)?.level).toBe(2);
    expect(tierFor(TIERS, 100)?.level).toBe(3);
    expect(tierFor(TIERS, 100_000)?.level).toBe(12);
  });

  it("has no tier below the smallest gift", () => {
    expect(tierFor(TIERS, 9)).toBeUndefined();
  });

  it("draws the top tier as a gradient", () => {
    expect(tierStyle(tier(12, 1n, ["#ff0000", "#00ff00"])).background).toBe(
      "linear-gradient(100deg, #ff0000, #00ff00)",
    );
    expect(tierStyle(tier(1, 1n)).background).toBe("#111111");
  });

  it("reads typed amounts", () => {
    expect(parseCoins("1,000")).toBe(1000n);
    expect(parseCoins(" 50 ")).toBe(50n);
    expect(parseCoins("0")).toBeNull();
    expect(parseCoins("12.5")).toBeNull();
    expect(parseCoins("abc")).toBeNull();
  });

  it("keeps the key only for a repeat of exactly the same gift", () => {
    let counter = 0;
    const next = () => `key-${++counter}-abcdefgh`;
    const first = giftAttempt(null, 10n, "がんばれ", next);
    expect(giftAttempt(first, 10n, "がんばれ", next)).toBe(first);
    expect(giftAttempt(first, 20n, "がんばれ", next).key).not.toBe(first.key);
    expect(giftAttempt(first, 10n, "ちがう", next).key).not.toBe(first.key);
    expect(giftAttempt(null, 10n, "がんばれ", next).key).not.toBe(first.key);
  });

  it("notices when the server answers with another gift", () => {
    const attempt = { key: "k", coins: 20n, message: "b" };
    expect(sameGift(attempt, { coins: 20n, message: "b" })).toBe(true);
    expect(sameGift(attempt, { coins: 10n, message: "a" })).toBe(false);
  });

  it("makes idempotency keys the server accepts", () => {
    const key = newIdempotencyKey(() => "0199aaaa-bbbb-7ccc-8ddd-eeeeeeeeeeee");
    expect(key).toMatch(/^[A-Za-z0-9_-]{8,64}$/);
  });
});

describe("chat list", () => {
  it("keeps lines in stream order and replaces a repeated one", () => {
    const first = addChatMessages([], [line("b", 2000), line("a", 1000)]);
    expect(first.map((message) => message.id)).toEqual(["a", "b"]);
    const again = addChatMessages(first, [line("b", 2000), line("c", 3000)]);
    expect(again.map((message) => message.id)).toEqual(["a", "b", "c"]);
  });

  it("forgets the oldest lines past the limit", () => {
    const lines = Array.from({ length: 5 }, (_, index) => line(`m${index}`, index));
    expect(addChatMessages([], lines, 3).map((message) => message.id)).toEqual(["m2", "m3", "m4"]);
  });

  it("removes one line, or everything a person wrote", () => {
    const lines = [line("a", 1, "u1"), line("b", 2, "u2"), line("c", 3, "u1")];
    expect(removeChatMessage(lines, "b").map((message) => message.id)).toEqual(["a", "c"]);
    expect(removeChatMessage(lines, "zz")).toBe(lines);
    expect(removeChatUser(lines, "u1").map((message) => message.id)).toEqual(["b"]);
  });

  it("shows only gifts that are still highlighted, biggest first", () => {
    const gifts = [
      line("a", 1, "u1", 100n, 200),
      line("b", 2, "u2", 500n, 200),
      line("c", 3, "u3", 900n, 50),
    ];
    expect(activeHighlights(gifts, 100_000).map((message) => message.id)).toEqual(["b", "a"]);
  });

  it("counts down slow mode", () => {
    expect(slowModeWait(30, 10_000, 15_000)).toBe(25);
    expect(slowModeWait(30, 10_000, 40_000)).toBe(0);
    expect(slowModeWait(0, 10_000, 11_000)).toBe(0);
    expect(slowModeWait(30, null, 11_000)).toBe(0);
  });
});

describe("polls", () => {
  const poll = (votes: bigint[], viewerChoice = 0) =>
    create(live.PollSchema, {
      id: "p1",
      options: votes.map((voteCount) => ({ text: "x", voteCount })),
      totalVotes: votes.reduce((sum, count) => sum + count, 0n),
      viewerChoice,
    });

  it("turns votes into percent", () => {
    expect(pollShares(poll([3n, 1n]))).toEqual([75, 25]);
    expect(pollShares(poll([0n, 0n]))).toEqual([0, 0]);
  });

  it("keeps the viewer's vote when a push arrives without it", () => {
    expect(mergePoll(poll([1n, 0n], 1), poll([2n, 1n])).viewerChoice).toBe(1);
    expect(mergePoll(undefined, poll([2n, 1n])).viewerChoice).toBe(0);
  });
});

describe("countdown", () => {
  it("splits what is left", () => {
    expect(countdownParts(90_061_000, 0)).toMatchObject({
      days: 1,
      hours: 1,
      minutes: 1,
      seconds: 1,
    });
    expect(countdownParts(0, 5000).total).toBe(0);
  });
});

describe("clip range", () => {
  it("never runs past a minute", () => {
    expect(clampClipRange(10, 200, 600, "end")).toEqual({ start: 140, end: 200 });
    expect(clampClipRange(10, 200, 600, "start")).toEqual({ start: 10, end: 70 });
  });

  it("stays inside the source and never collapses", () => {
    expect(clampClipRange(-5, 3, 20, "start")).toEqual({ start: 0, end: 3 });
    expect(clampClipRange(15, 99, 20, "end")).toEqual({ start: 15, end: 20 });
    expect(clampClipRange(8, 8, 20, "start")).toEqual({ start: 8, end: 8.1 });
    expect(clampClipRange(8, 2, 20, "end")).toEqual({ start: 1.9, end: 2 });
  });
});

describe("secrets", () => {
  it("masks without leaking the length", () => {
    expect(maskSecret("abc")).toBe(maskSecret("a-much-longer-secret"));
    expect(maskSecret("")).toBe("");
  });

  it("keeps the host of a URL readable and hides the rest", () => {
    const masked = maskUrl("rtmp://live.example:1935/live/abc?token=SECRET");
    expect(masked.startsWith("rtmp://live.example:1935/")).toBe(true);
    expect(masked).not.toContain("SECRET");
    expect(masked).not.toContain("abc");
  });
});

describe("alert text", () => {
  it("fills the placeholders", () => {
    expect(
      fillAlertTemplate("{name} が {coins} コイン! {message} ({count})", {
        name: "ミナ",
        coins: "500",
        message: "がんばれ",
        count: "3",
      }),
    ).toBe("ミナ が 500 コイン! がんばれ (3)");
  });
});
