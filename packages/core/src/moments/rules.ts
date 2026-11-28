import { type common, moments, toMillis } from "@tokuly/api";

/**
 * Where the caller's day stands, for the screen that takes the moment:
 * - `waiting`: no prompt has reached them yet (the time is a secret until it does);
 * - `open`: the window is running, `leftMs` of it left;
 * - `late`: the window is over and nothing was posted; posting still works, and says how late;
 * - `posted`: today's moment is out (also when deleted since: there is no second one).
 */
export type MomentPhase =
  | { kind: "waiting" }
  | { kind: "open"; leftMs: number }
  | { kind: "late" }
  | { kind: "posted" };

export function momentPhase(
  today: Pick<moments.GetTodayResponse, "day" | "posted"> | undefined,
  nowMs: number,
): MomentPhase {
  const day = today?.day;
  if (!day) return { kind: "waiting" };
  if (today.posted) return { kind: "posted" };
  const leftMs = toMillis(day.onTimeUntil) - nowMs;
  return leftMs > 0 ? { kind: "open", leftMs } : { kind: "late" };
}

/**
 * What a moment's picture may show. A locked moment comes without its
 * pictures' addresses: only the blurred placeholder (ThumbHash) and the
 * colour, until the viewer shares their own moment of the day.
 */
export type MomentPicture =
  | { kind: "picture"; url: string; image: common.Image }
  | { kind: "blur"; thumbhash: string; color: string };

export function momentPicture(
  image: common.Image | undefined,
  locked: boolean,
): MomentPicture | null {
  if (!image) return null;
  if (locked || !image.url)
    return { kind: "blur", thumbhash: image.thumbhash, color: image.dominantColor };
  return { kind: "picture", url: image.url, image };
}

/** Why the viewer cannot react to a moment, or `null` when they can. */
export function reactionBlocker(
  moment: Pick<moments.Moment, "locked" | "mine">,
): "locked" | "own" | null {
  if (moment.mine) return "own";
  if (moment.locked) return "locked";
  return null;
}

/** The faces of the RealMoji library, in the order the apps show them. */
export const REALMOJI_FACES = [
  moments.RealmojiEmoji.THUMBS_UP,
  moments.RealmojiEmoji.SMILE,
  moments.RealmojiEmoji.SURPRISED,
  moments.RealmojiEmoji.HEART_EYES,
  moments.RealmojiEmoji.LAUGHING,
] as const;

const GLYPHS: Record<number, string> = {
  [moments.RealmojiEmoji.THUMBS_UP]: "👍",
  [moments.RealmojiEmoji.SMILE]: "😃",
  [moments.RealmojiEmoji.SURPRISED]: "😮",
  [moments.RealmojiEmoji.HEART_EYES]: "😍",
  [moments.RealmojiEmoji.LAUGHING]: "😂",
  [moments.RealmojiEmoji.INSTANT]: "⚡",
};

/** The emoji a RealMoji face stands for (⚡ for one taken on the spot). */
export function realmojiGlyph(emoji: moments.RealmojiEmoji): string {
  return GLYPHS[emoji] ?? "";
}

/** A caption is one line of at most this many characters. */
export const CAPTION_MAX = 100;

/**
 * The caption as the API takes it: one line without control characters
 * (line breaks and tabs become a space), trimmed; `null` when it is too long.
 */
export function cleanCaption(text: string): string | null {
  const line = text.replace(/\s*\p{Cc}+\s*/gu, " ").trim();
  return [...line].length > CAPTION_MAX ? null : line;
}
