import { create, fromDate, moments } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import { cleanCaption, momentPhase, momentPicture, reactionBlocker, realmojiGlyph } from "./rules";

const NOW = new Date(2026, 10, 28, 12, 0, 0).getTime();
const day = (untilMs: number) =>
  create(moments.MomentDaySchema, { date: "2026-11-28", onTimeUntil: fromDate(new Date(untilMs)) });

describe("momentPhase", () => {
  it("waits until a prompt reached the caller", () => {
    expect(momentPhase(undefined, NOW)).toEqual({ kind: "waiting" });
    expect(momentPhase({ day: undefined, posted: false }, NOW)).toEqual({ kind: "waiting" });
  });

  it("counts the window down, then calls it late", () => {
    expect(momentPhase({ day: day(NOW + 90_000), posted: false }, NOW)).toEqual({
      kind: "open",
      leftMs: 90_000,
    });
    expect(momentPhase({ day: day(NOW), posted: false }, NOW)).toEqual({ kind: "late" });
  });

  it("is posted once a moment of the day went out, deleted or not", () => {
    expect(momentPhase({ day: day(NOW + 90_000), posted: true }, NOW)).toEqual({ kind: "posted" });
  });
});

describe("momentPicture", () => {
  const image = {
    url: "/media/a.jpg",
    thumbhash: "1QcSHQRnh493V4dIh4eXh1h4kJUI",
    dominantColor: "#334455",
  };

  it("shows only the placeholder of a locked moment", () => {
    expect(momentPicture(image as never, true)).toEqual({
      kind: "blur",
      thumbhash: image.thumbhash,
      color: "#334455",
    });
    expect(momentPicture({ ...image, url: "" } as never, false)?.kind).toBe("blur");
  });

  it("shows the picture once unlocked", () => {
    expect(momentPicture(image as never, false)).toMatchObject({ kind: "picture", url: image.url });
    expect(momentPicture(undefined, false)).toBeNull();
  });
});

describe("reactions", () => {
  it("are for unlocked moments of others", () => {
    expect(reactionBlocker({ mine: true, locked: false })).toBe("own");
    expect(reactionBlocker({ mine: false, locked: true })).toBe("locked");
    expect(reactionBlocker({ mine: false, locked: false })).toBeNull();
  });

  it("name each face by its emoji", () => {
    expect(realmojiGlyph(moments.RealmojiEmoji.HEART_EYES)).toBe("😍");
    expect(realmojiGlyph(moments.RealmojiEmoji.INSTANT)).toBe("⚡");
    expect(realmojiGlyph(moments.RealmojiEmoji.UNSPECIFIED)).toBe("");
  });
});

describe("cleanCaption", () => {
  it("keeps one trimmed line of at most 100 characters", () => {
    expect(cleanCaption("  渋谷なう\nたのしい ")).toBe("渋谷なう たのしい");
    expect(cleanCaption("🍣".repeat(100))).toBe("🍣".repeat(100));
    expect(cleanCaption("あ".repeat(101))).toBeNull();
  });
});
