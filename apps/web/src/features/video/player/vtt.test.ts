import { describe, expect, it } from "vitest";
import { activeCues, frameAt, parseStoryboard, parseVtt } from "./vtt";

const SUBTITLES = `WEBVTT - made for a test

NOTE this block is a comment

1
00:00:01.000 --> 00:00:04.000 line:90% align:center
<v Yuki>こんにちは！</v>
二行目 &amp; more

00:05.500 --> 00:07.250
<i>short</i> form

broken block without timing

00:00:09.000 --> 00:00:08.000
ends before it starts
`;

describe("WebVTT", () => {
  it("reads cues, dropping markup, notes and malformed blocks", () => {
    expect(parseVtt(SUBTITLES)).toEqual([
      { start: 1, end: 4, text: "こんにちは！\n二行目 & more" },
      { start: 5.5, end: 7.25, text: "short form" },
    ]);
  });

  it("copes with Windows line endings and a byte-order mark", () => {
    expect(parseVtt("﻿WEBVTT\r\n\r\n00:00:00.000 --> 00:00:01.000\r\nhi\r\n")).toEqual([{ start: 0, end: 1, text: "hi" }]);
  });

  it("finds the cues on screen at a moment", () => {
    const cues = parseVtt(SUBTITLES);
    expect(activeCues(cues, 0.5)).toEqual([]);
    expect(activeCues(cues, 1).map((cue) => cue.start)).toEqual([1]);
    expect(activeCues(cues, 4)).toEqual([]);
    expect(activeCues(cues, 6).map((cue) => cue.start)).toEqual([5.5]);
  });

  it("reads a seek-preview track relative to its own address", () => {
    const track = "WEBVTT\n\n00:00:00.000 --> 00:00:10.000\nsprite_001.webp#xywh=0,0,160,90\n\n00:00:10.000 --> 00:00:20.000\nsprite_001.webp#xywh=160,0,160,90\n\n00:00:20.000 --> 00:00:30.000\nnot a frame\n";
    const frames = parseStoryboard(track, "https://media.example/video/abc/storyboard/storyboard.vtt");
    expect(frames).toEqual([
      { start: 0, end: 10, url: "https://media.example/video/abc/storyboard/sprite_001.webp", x: 0, y: 0, width: 160, height: 90 },
      { start: 10, end: 20, url: "https://media.example/video/abc/storyboard/sprite_001.webp", x: 160, y: 0, width: 160, height: 90 },
    ]);
    expect(frameAt(frames, 3)?.x).toBe(0);
    expect(frameAt(frames, 10)?.x).toBe(160);
    // Past the last frame, the last one stands in.
    expect(frameAt(frames, 45)?.x).toBe(160);
    expect(frameAt([], 3)).toBeUndefined();
  });
});
