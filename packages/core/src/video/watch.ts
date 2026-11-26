import { common, video } from "@tokuly/api";
import { parseClock, parseTimeParam } from "./format";

const { PlaybackBlock, ProcessingState, Rating, PlaylistKind } = video;

/** What a watch link carries: `/watch/<id>?t=90&list=<playlist>&comment=<id>`. */
export interface WatchSearch {
  /** Where to start: seconds, `1:30` or `1m30s`. */
  t?: string;
  /** The playlist being played through. */
  list?: string;
  /** A comment to open the thread at. */
  comment?: string;
}

/** Reads a watch link's search; anything that is not a non-empty value is left out. */
export function validateWatchSearch(search: Record<string, unknown>): WatchSearch {
  const text = (value: unknown) =>
    typeof value === "string" && value
      ? value
      : typeof value === "number"
        ? String(value)
        : undefined;
  const result: WatchSearch = {};
  const t = text(search.t);
  const list = text(search.list);
  const comment = text(search.comment);
  if (t) result.t = t;
  if (list) result.list = list;
  if (comment) result.comment = comment;
  return result;
}

/** Where playback begins: the link's `?t=`, else where the viewer left off (unless that is the very end). */
export function startPosition(
  param: string | undefined,
  summary: Pick<video.VideoSummary, "durationSeconds" | "resumePositionSeconds" | "watched">,
): number {
  const asked = parseTimeParam(param);
  if (asked !== null) return Math.min(asked, Math.max(0, summary.durationSeconds - 1));
  const resume = summary.resumePositionSeconds;
  return summary.watched || resume < 3 || resume > summary.durationSeconds - 5 ? 0 : resume;
}

/** The chapter playing at `time` (chapters are sorted by start). */
export function chapterAt(chapters: video.Chapter[], time: number): video.Chapter | undefined {
  let current: video.Chapter | undefined;
  for (const chapter of chapters) {
    if (chapter.startSeconds <= time + 0.001) current = chapter;
    else break;
  }
  return current;
}

/** A piece of a creator's free text: plain, a web address, or a clock time to jump to. */
export type DescriptionPart =
  | { kind: "text"; text: string }
  | { kind: "link"; text: string }
  | { kind: "time"; text: string; seconds: number };

// A web address runs as far as characters that belong in one: Japanese text often follows it without a space.
const PATTERN =
  /(https?:\/\/[A-Za-z0-9\-._~:/?#[\]@!$&'()*+,;=%]+)|\b(\d{1,2}:\d{2}(?::\d{2})?)\b/g;
const TRAILING = /[.,!?;:)\]']+$/;

/** A description (or a comment) cut into text, web addresses and clock times (`1:23`). */
export function descriptionParts(text: string): DescriptionPart[] {
  const parts: DescriptionPart[] = [];
  let last = 0;
  for (const match of text.matchAll(PATTERN)) {
    const start = match.index;
    let token = match[0];
    let part: DescriptionPart;
    if (match[1]) {
      token = token.replace(TRAILING, "");
      part = { kind: "link", text: token };
    } else {
      const seconds = parseClock(token);
      if (seconds === null) continue;
      part = { kind: "time", text: token, seconds };
    }
    if (start > last) parts.push({ kind: "text", text: text.slice(last, start) });
    parts.push(part);
    last = start + token.length;
  }
  if (last < text.length) parts.push({ kind: "text", text: text.slice(last) });
  return parts;
}

/**
 * `watch.audience.<key>` of the line under a title saying who can see the
 * video; `undefined` when it is plainly public. An audience this client does
 * not know reads as private.
 */
export function audienceKey(visibility: common.Visibility, allowLink: boolean): string | undefined {
  if (visibility === common.Visibility.PUBLIC) return undefined;
  const key = {
    [common.Visibility.NO_FEED]: "noFeed",
    [common.Visibility.UNLISTED]: "unlisted",
    [common.Visibility.FRIENDS]: allowLink ? "friendsOrLink" : "friends",
    [common.Visibility.PRIVATE]: "private",
    [common.Visibility.FOLLOWERS]: "followers",
    [common.Visibility.CLOSE_FRIENDS]: "closeFriends",
    [common.Visibility.CIRCLE]: "circle",
  }[visibility as number];
  return `watch.audience.${key ?? "private"}`;
}

/** Why the player is not there, as the `watch.blocked.<kind>Title` / `…Body` strings name it. */
export type BlockKind = "age" | "signIn" | "failed" | "processing";

export function blockKind(item: Pick<video.Video, "playbackBlock" | "summary">): BlockKind {
  if (item.playbackBlock === PlaybackBlock.AGE_RESTRICTED) return "age";
  if (item.playbackBlock === PlaybackBlock.SIGN_IN_REQUIRED) return "signIn";
  return item.summary?.processing === ProcessingState.FAILED ? "failed" : "processing";
}

/** The video with the viewer's rating changed, as the thumb shows it before the server answers. */
export function withRating(item: video.Video, next: video.Rating): video.Video {
  const before = item.viewer?.rating ?? Rating.NONE;
  return {
    ...item,
    viewer: item.viewer && { ...item.viewer, rating: next },
    summary: item.summary && {
      ...item.summary,
      likeCount:
        item.summary.likeCount +
        (next === Rating.LIKE ? 1n : 0n) -
        (before === Rating.LIKE ? 1n : 0n),
    },
  };
}

/** A playlist's name: the built-in ones are named by the app (`t` is the video namespace's). */
export function playlistTitle(
  playlist: Pick<video.Playlist, "kind" | "title">,
  t: (key: string) => string,
): string {
  if (playlist.kind === PlaylistKind.WATCH_LATER) return t("playlist.watchLater");
  if (playlist.kind === PlaylistKind.LIKED) return t("playlist.liked");
  return playlist.title;
}

/** The video after `currentId` in a playlist; `undefined` at its end or while it is not in the loaded part. */
export function nextInQueue(
  items: video.VideoSummary[],
  currentId: string | undefined,
): video.VideoSummary | undefined {
  const index = items.findIndex((item) => item.id === currentId);
  return index >= 0 ? items[index + 1] : undefined;
}

/** The best-fitting URL of an image for a box `pixels` wide (twice that, for sharp screens). */
export function imageUrl(image: common.Image | undefined, pixels: number): string | undefined {
  if (!image) return undefined;
  const fitting = image.variants
    .filter((variant) => variant.size >= pixels * 2)
    .sort((a, b) => a.size - b.size)[0];
  return fitting?.url ?? image.url ?? undefined;
}
