/** One cue of a WebVTT file. */
export interface Cue {
  start: number;
  end: number;
  text: string;
}

/** A frame of the seek-preview track: a region of a sprite sheet. */
export interface StoryboardFrame {
  start: number;
  end: number;
  url: string;
  x: number;
  y: number;
  width: number;
  height: number;
}

/** `00:01:02.345` or `01:02.345` → seconds. */
function parseTimestamp(text: string): number | null {
  const match = /^(?:(\d+):)?(\d{1,2}):(\d{2})[.,](\d{3})$/.exec(text.trim());
  if (!match) return null;
  return (
    Number(match[1] ?? 0) * 3600 +
    Number(match[2]) * 60 +
    Number(match[3]) +
    Number(match[4]) / 1000
  );
}

const ENTITIES: Record<string, string> = {
  "&amp;": "&",
  "&lt;": "<",
  "&gt;": ">",
  "&nbsp;": " ",
  "&lrm;": "",
  "&rlm;": "",
};

/** Cue text without markup: `<v Name>`, `<i>`, `<c.class>`, inline timestamps. */
function plainText(text: string): string {
  return text
    .replace(/<[^>]*>/g, "")
    .replace(/&(?:amp|lt|gt|nbsp|lrm|rlm);/g, (entity) => ENTITIES[entity] ?? entity);
}

/**
 * Reads the cues of a WebVTT file, in file order. Notes, styles, regions and
 * malformed blocks are skipped; cue settings (position, alignment) are ignored.
 */
export function parseVtt(source: string): Cue[] {
  const cues: Cue[] = [];
  const blocks = source
    .replace(/^﻿/, "")
    .replace(/\r\n?/g, "\n")
    .split(/\n{2,}/);
  for (const block of blocks) {
    const lines = block.split("\n").filter((line) => line.trim() !== "");
    const timing = lines.findIndex((line) => line.includes("-->"));
    // The line before the timing, if any, is the cue's identifier.
    if (timing < 0 || timing > 1) continue;
    const [from, to] = (lines[timing] ?? "").split("-->");
    const start = parseTimestamp(from ?? "");
    const end = parseTimestamp((to ?? "").trim().split(/\s+/)[0] ?? "");
    if (start === null || end === null || end <= start) continue;
    const text = plainText(lines.slice(timing + 1).join("\n")).trim();
    if (text) cues.push({ start, end, text });
  }
  return cues;
}

/** The cues on screen at `time`, in file order. */
export function activeCues(cues: Cue[], time: number): Cue[] {
  return cues.filter((cue) => time >= cue.start && time < cue.end);
}

/**
 * Reads a seek-preview track: each cue names a sprite sheet and a region of
 * it (`sprite_001.webp#xywh=160,0,160,90`), relative to the track's own URL.
 */
export function parseStoryboard(source: string, trackUrl: string): StoryboardFrame[] {
  const frames: StoryboardFrame[] = [];
  for (const cue of parseVtt(source)) {
    const match = /^(.*)#xywh=(\d+),(\d+),(\d+),(\d+)$/.exec(cue.text);
    if (!match) continue;
    let url: string;
    try {
      url = new URL(match[1] ?? "", trackUrl).href;
    } catch {
      continue;
    }
    frames.push({
      start: cue.start,
      end: cue.end,
      url,
      x: Number(match[2]),
      y: Number(match[3]),
      width: Number(match[4]),
      height: Number(match[5]),
    });
  }
  return frames;
}

/** The preview frame for `time`; the last one stands in for anything past the end. */
export function frameAt(frames: StoryboardFrame[], time: number): StoryboardFrame | undefined {
  return (
    frames.find((frame) => time >= frame.start && time < frame.end) ??
    (frames.length > 0 && time >= (frames.at(-1)?.end ?? 0) ? frames.at(-1) : undefined)
  );
}
