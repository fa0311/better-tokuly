import { moments } from "@tokuly/api";

const NAMES: Record<number, string> = {
  [moments.RealmojiEmoji.THUMBS_UP]: "thumbsUp",
  [moments.RealmojiEmoji.SMILE]: "smile",
  [moments.RealmojiEmoji.SURPRISED]: "surprised",
  [moments.RealmojiEmoji.HEART_EYES]: "heartEyes",
  [moments.RealmojiEmoji.LAUGHING]: "laughing",
  [moments.RealmojiEmoji.INSTANT]: "instant",
};

/** The `moments:realmoji.*` key naming a face. */
export const faceKey = (emoji: moments.RealmojiEmoji) =>
  `realmoji.${NAMES[emoji] ?? "instant"}` as const;
