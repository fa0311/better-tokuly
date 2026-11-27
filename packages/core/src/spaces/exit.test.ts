import { create } from "@bufbuild/protobuf";
import { spaces } from "@tokuly/api";
import { describe, expect, it } from "vitest";
import { exitReasonOf, voicesOf, whyOutsideHangout, whyOutsideSpace } from "./exit";

const room = (state: spaces.SpaceState, inside: boolean) =>
  create(spaces.SpaceSchema, { id: "s1", state, viewer: { inside } });

describe("exitReasonOf", () => {
  it("says the server's reason in the apps' words", () => {
    expect(exitReasonOf(spaces.SpaceLeaveReason.BANNED)).toBe("banned");
    expect(exitReasonOf(spaces.SpaceLeaveReason.DISCONNECTED)).toBe("lost");
    expect(exitReasonOf(spaces.SpaceLeaveReason.LEFT)).toBe("elsewhere");
  });
});

describe("whyOutsideSpace", () => {
  it("is still inside a live room", () => {
    expect(whyOutsideSpace(room(spaces.SpaceState.LIVE, true), true)).toBeNull();
  });

  it("is outside a room that is gone or over", () => {
    expect(whyOutsideSpace(undefined, false)).toBe("removed");
    expect(whyOutsideSpace(room(spaces.SpaceState.ENDED, true), false)).toBe("ended");
  });

  it("is removed only when a moderator cut the connection", () => {
    expect(whyOutsideSpace(room(spaces.SpaceState.LIVE, false), true)).toBe("removed");
    expect(whyOutsideSpace(room(spaces.SpaceState.LIVE, false), false)).toBeNull();
  });
});

describe("whyOutsideHangout", () => {
  it("is outside a hangout they are no member of", () => {
    const member = create(spaces.HangoutSchema, {
      viewer: { state: spaces.HangoutMemberState.MEMBER },
    });
    expect(whyOutsideHangout(member)).toBeNull();
    expect(whyOutsideHangout(create(spaces.HangoutSchema, {}))).toBe("closed");
    expect(whyOutsideHangout(undefined)).toBe("closed");
  });
});

describe("voicesOf", () => {
  it("lights a voice that went quiet a moment ago, never a closed microphone", () => {
    const voices = voicesOf(
      [
        { identity: "a", isSpeaking: true, isMicrophoneEnabled: true },
        { identity: "b", isSpeaking: false, isMicrophoneEnabled: true },
        { identity: "c", isSpeaking: true, isMicrophoneEnabled: false },
        { identity: "d", isSpeaking: false, isMicrophoneEnabled: true },
      ],
      new Map([
        ["b", 9_000],
        ["d", 5_000],
      ]),
      10_000,
    );
    expect([...voices.speaking]).toEqual(["a", "b"]);
    expect([...voices.open]).toEqual(["a", "b", "d"]);
    expect(voices.connected.size).toBe(4);
  });
});
