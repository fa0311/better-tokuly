import { common, shorts } from "@tokuly/api";
import type { FeedContext } from "./search";

const { PlaybackSource } = shorts;

/** The longest short, in seconds, and the widest picture (width / height) the server accepts. */
export const MAX_SECONDS = 180;
export const MAX_ASPECT = 16 / 9;
export const MAX_CAPTION = 500;

/** A short as the shared controls (reactions, comments, report, share to chat) know it. */
export function shortSubject(short: Pick<shorts.Short, "id">): {
  type: common.SubjectType;
  id: string;
} {
  return { type: common.SubjectType.SHORT, id: short.id };
}

// ───────────────────────────── Captions ─────────────────────────────

export type CaptionPart =
  | { kind: "text"; text: string }
  | { kind: "hashtag"; text: string; tag: string }
  | { kind: "mention"; text: string; handle: string };

const TOKEN = /([#＃][\p{L}\p{N}_]+)|(@[A-Za-z0-9_]+)/gu;

/**
 * A caption cut into plain text, hashtags and mentions. Only the hashtags
 * and mentions the server read out of it become links: an `@name` that names
 * nobody stays text.
 */
export function captionParts(
  caption: string,
  hashtags: readonly string[],
  mentions: readonly Pick<shorts.Mention, "handle">[],
): CaptionPart[] {
  const tags = new Set(hashtags);
  const handles = new Set(mentions.map((mention) => mention.handle));
  const parts: CaptionPart[] = [];
  const text = (value: string) => {
    if (!value) return;
    const last = parts[parts.length - 1];
    if (last?.kind === "text") last.text += value;
    else parts.push({ kind: "text", text: value });
  };
  let cursor = 0;
  for (const match of caption.matchAll(TOKEN)) {
    const raw = match[0];
    const index = match.index;
    text(caption.slice(cursor, index));
    cursor = index + raw.length;
    const name = raw.slice(1).toLowerCase();
    if (match[1] && tags.has(name)) parts.push({ kind: "hashtag", text: raw, tag: name });
    else if (match[2] && handles.has(name))
      parts.push({ kind: "mention", text: raw, handle: name });
    else text(raw);
  }
  text(caption.slice(cursor));
  return parts;
}

/** The `#tag` or `@name` being typed at the caret, for suggestions while writing a caption. */
export function tokenAtCaret(
  text: string,
  caret: number,
): { kind: "hashtag" | "mention"; query: string; start: number } | null {
  const before = text.slice(0, caret);
  const match = /(^|[\s　])([#＃@])([\p{L}\p{N}_]*)$/u.exec(before);
  if (!match) return null;
  const query = match[3] ?? "";
  const kind = match[2] === "@" ? "mention" : "hashtag";
  if (kind === "mention" && !/^[A-Za-z0-9_]*$/.test(query)) return null;
  return { kind, query, start: caret - query.length - 1 };
}

/** The caption with the token at `start` replaced by a picked suggestion, and where the caret goes. */
export function completeToken(
  text: string,
  start: number,
  caret: number,
  replacement: string,
): { text: string; caret: number } {
  const head = `${text.slice(0, start)}${replacement} `;
  return { text: head + text.slice(caret).replace(/^ /, ""), caret: head.length };
}

// ───────────────────────────── Where a short is being watched ─────────────────────────────

/** What the playback report says about where the viewer was. */
export function playbackSource(context: FeedContext): shorts.PlaybackSource {
  switch (context.kind) {
    case "forYou":
      return PlaybackSource.FOR_YOU;
    case "following":
      return PlaybackSource.FOLLOWING;
    case "friends":
      return PlaybackSource.FRIENDS;
    case "channel":
    case "user":
      return PlaybackSource.PROFILE;
    case "hashtag":
      return PlaybackSource.HASHTAG;
    case "sound":
      return PlaybackSource.SOUND;
    case "link":
      return context.via === "search"
        ? PlaybackSource.SEARCH
        : context.via === "notification"
          ? PlaybackSource.NOTIFICATION
          : PlaybackSource.SHARE_LINK;
    default:
      return PlaybackSource.OTHER;
  }
}

// ───────────────────────────── The window of live slides ─────────────────────────────

/**
 * What a slide may hold, by its distance from the one on screen. Only the
 * current short and the next one own a `<video>` (the next one is loading
 * ahead); the previous one keeps its cover so swiping back shows a picture
 * at once; everything further away is an empty box.
 */
export type SlideLoad = "playing" | "preloading" | "cover" | "empty";

export function slideLoad(index: number, current: number): SlideLoad {
  if (index === current) return "playing";
  if (index === current + 1) return "preloading";
  if (index === current - 1 || index === current + 2) return "cover";
  return "empty";
}

// ───────────────────────────── Small formatting ─────────────────────────────

/** 83 → "1:23". */
export function clock(seconds: number): string {
  const whole = Math.max(0, Math.round(seconds));
  return `${Math.floor(whole / 60)}:${String(whole % 60).padStart(2, "0")}`;
}

/** The smallest stored rendition of a picture that still covers `pixels` on a 2x screen. */
export function imageUrl(image: common.Image | undefined, pixels: number): string | undefined {
  if (!image) return undefined;
  const fitting = image.variants
    .filter((variant) => variant.size >= pixels * 2)
    .sort((a, b) => a.size - b.size)[0];
  return fitting?.url ?? image.url ?? undefined;
}

/** A sound's name as shown next to the note: the track, or "original sound" with whose it is. */
export function soundTitle(
  sound: Pick<shorts.Sound, "kind" | "title" | "artist" | "channel">,
  originalLabel: (channel: string) => string,
): string {
  if (sound.kind === shorts.SoundKind.EXTERNAL)
    return sound.artist ? `${sound.title} · ${sound.artist}` : sound.title;
  const name = sound.channel?.name ?? "";
  return sound.title ? (name ? `${sound.title} · ${name}` : sound.title) : originalLabel(name);
}
