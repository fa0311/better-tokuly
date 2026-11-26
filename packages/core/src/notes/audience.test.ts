import { common } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import {
  audienceFields,
  audienceOption,
  noteAudience,
  replyAudience,
  sameAudience,
} from "./audience";

const { Visibility } = common;

describe("audienceOption", () => {
  it("tells a direct note from one only the author sees", () => {
    expect(audienceOption({ visibility: Visibility.PRIVATE }).key).toBe("private");
    expect(audienceOption({ visibility: Visibility.PRIVATE, directUserIds: ["u"] }).key).toBe(
      "direct",
    );
    expect(audienceOption({ visibility: Visibility.PRIVATE, direct: true }).key).toBe("direct");
  });

  it("reads a value it does not know as the narrowest, never as public", () => {
    expect(audienceOption({ visibility: 99 as common.Visibility }).key).toBe("private");
  });
});

describe("replyAudience", () => {
  const publicNote = { visibility: Visibility.PUBLIC };
  it("keeps what was chosen when the answered note is as wide", () => {
    expect(replyAudience({ visibility: Visibility.FRIENDS }, publicNote).key).toBe("friends");
    expect(replyAudience({ visibility: Visibility.PUBLIC }, undefined).key).toBe("public");
  });

  it("never answers wider than the note it answers", () => {
    expect(
      replyAudience({ visibility: Visibility.PUBLIC }, { visibility: Visibility.FOLLOWERS }).key,
    ).toBe("followers");
  });

  it("answers close friends, a circle or named people directly", () => {
    for (const visibility of [Visibility.CLOSE_FRIENDS, Visibility.CIRCLE])
      expect(replyAudience({ visibility: Visibility.PUBLIC }, { visibility }).key).toBe("direct");
    const direct = noteAudience({
      visibility: Visibility.PRIVATE,
      directRecipients: [{ id: "u" }],
    });
    expect(replyAudience({ visibility: Visibility.FRIENDS }, direct).key).toBe("direct");
  });
});

describe("audiences as the API takes them", () => {
  it("leaves the default to the account when nothing was chosen", () => {
    expect(audienceFields(undefined)).toEqual({
      visibility: Visibility.UNSPECIFIED,
      circleId: "",
      directUserIds: [],
    });
  });

  it("compares people regardless of their order", () => {
    expect(
      sameAudience(
        { visibility: Visibility.PRIVATE, directUserIds: ["a", "b"] },
        { visibility: Visibility.PRIVATE, directUserIds: ["b", "a"] },
      ),
    ).toBe(true);
  });
});
