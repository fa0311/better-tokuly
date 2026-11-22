import { describe, expect, it } from "vitest";
import { commandForKey, stepRate } from "./shortcuts";

describe("player shortcuts", () => {
  it("maps the keys people know from other video sites", () => {
    expect(commandForKey({ key: " " })).toEqual({ type: "toggle" });
    expect(commandForKey({ key: "k" })).toEqual({ type: "toggle" });
    expect(commandForKey({ key: "j" })).toEqual({ type: "seekBy", seconds: -10 });
    expect(commandForKey({ key: "l" })).toEqual({ type: "seekBy", seconds: 10 });
    expect(commandForKey({ key: "ArrowLeft" })).toEqual({ type: "seekBy", seconds: -5 });
    expect(commandForKey({ key: "ArrowRight" })).toEqual({ type: "seekBy", seconds: 5 });
    expect(commandForKey({ key: "ArrowUp" })).toEqual({ type: "volumeBy", delta: 0.05 });
    expect(commandForKey({ key: "f" })).toEqual({ type: "fullscreen" });
    expect(commandForKey({ key: "M" })).toEqual({ type: "mute" });
    expect(commandForKey({ key: "t" })).toEqual({ type: "theatre" });
    expect(commandForKey({ key: "c" })).toEqual({ type: "captions" });
    expect(commandForKey({ key: ">" , shiftKey: true })).toEqual({ type: "rateBy", delta: 0.25 });
  });

  it("jumps to tenths of the video with the digits", () => {
    expect(commandForKey({ key: "0" })).toEqual({ type: "seekToFraction", fraction: 0 });
    expect(commandForKey({ key: "7" })).toEqual({ type: "seekToFraction", fraction: 0.7 });
    // A layout that needs Shift for digits still reports the key's position.
    expect(commandForKey({ key: "è", code: "Digit7" })).toEqual({ type: "seekToFraction", fraction: 0.7 });
    expect(commandForKey({ key: "Home" })).toEqual({ type: "seekToFraction", fraction: 0 });
  });

  it("leaves browser shortcuts and other keys alone", () => {
    expect(commandForKey({ key: "f", ctrlKey: true })).toBeNull();
    expect(commandForKey({ key: "l", metaKey: true })).toBeNull();
    expect(commandForKey({ key: "ArrowLeft", altKey: true })).toBeNull();
    expect(commandForKey({ key: "x" })).toBeNull();
    expect(commandForKey({ key: "Tab" })).toBeNull();
  });

  it("steps the speed inside what the menu offers", () => {
    expect(stepRate(1, 0.25)).toBe(1.25);
    expect(stepRate(1, -0.25)).toBe(0.75);
    expect(stepRate(2, 0.25)).toBe(2);
    expect(stepRate(0.25, -0.25)).toBe(0.25);
  });
});
