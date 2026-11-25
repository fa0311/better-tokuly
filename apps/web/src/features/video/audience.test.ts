import { common } from "@tokuly/api";
import enShorts from "@tokuly/i18n/en/shorts";
import jaShorts from "@tokuly/i18n/ja/shorts";
import { describe, expect, it } from "vitest";
import enLive from "#/i18n/en/live";
import enVideo from "#/i18n/en/video";
import jaLive from "#/i18n/ja/live";
import jaVideo from "#/i18n/ja/video";
import { VISIBILITIES as STREAM_VISIBILITIES } from "../live/labels";
import { circleOf, DEFAULT_SETTINGS, visibilityKey } from "../shorts/studio/settings-fields";
import { circleChoice } from "./audience";
import { audienceNote } from "./description";
import { VIDEO_VISIBILITIES, visibilityOption } from "./labels";

const { Visibility } = common;
const AUDIENCES = [Visibility.FOLLOWERS, Visibility.CLOSE_FRIENDS, Visibility.CIRCLE];

describe("the social graph's audiences in the studios", () => {
  it("are offered for videos and streams, with their words in both languages", () => {
    for (const visibility of AUDIENCES) {
      const option = visibilityOption(visibility);
      expect(option.value).toBe(visibility);
      expect(VIDEO_VISIBILITIES).toContain(option);
      const stream = STREAM_VISIBILITIES.find((candidate) => candidate.value === visibility);
      expect(stream?.key).toBe(option.key);
      for (const strings of [jaVideo, enVideo, jaLive, enLive]) {
        const words = strings.visibility as Record<string, string>;
        expect(words[option.key]).toBeTruthy();
        expect(words[`${option.key}Hint`]).toBeTruthy();
      }
    }
  });

  it("are offered for shorts, and a circle goes with its circle only", () => {
    for (const visibility of AUDIENCES) {
      const key = visibilityKey(visibility);
      expect(key).not.toBe("private");
      for (const strings of [jaShorts, enShorts]) {
        expect((strings.visibility as Record<string, { label: string }>)[key]?.label).toBeTruthy();
      }
    }
    expect(circleOf({ ...DEFAULT_SETTINGS, visibility: Visibility.CIRCLE, circleId: "c" })).toBe(
      "c",
    );
    expect(circleOf({ ...DEFAULT_SETTINGS, visibility: Visibility.FOLLOWERS, circleId: "c" })).toBe(
      "",
    );
  });

  it("are named under the title of a video", () => {
    const t = (key: string) => key;
    expect(audienceNote(Visibility.FOLLOWERS, false, t)).toBe("watch.audience.followers");
    expect(audienceNote(Visibility.CLOSE_FRIENDS, false, t)).toBe("watch.audience.closeFriends");
    expect(audienceNote(99 as common.Visibility, false, t)).toBe("watch.audience.private");
    expect(jaVideo.watch.audience.followers).toBeTruthy();
  });

  it("never swaps a deleted circle for another one", () => {
    expect(circleChoice("", ["a", "b"])).toEqual({ autoPick: "a", selected: "", missing: false });
    expect(circleChoice("b", ["a", "b"])).toEqual({
      autoPick: undefined,
      selected: "b",
      missing: false,
    });
    expect(circleChoice("gone", ["a", "b"])).toEqual({
      autoPick: undefined,
      selected: "",
      missing: true,
    });
    expect(circleChoice("gone", undefined)).toEqual({ selected: "gone", missing: false });
  });
});
