import { common, live, video } from "@tokuly/api";
import { EyeOff, Globe, Link2, Lock, Users } from "lucide-react";
import type { ComponentType } from "react";

const { VideoCategory } = video;
const { Visibility } = common;

/** The genres a stream can have, in the order chips and menus list them. `key` is `category.<key>` in the strings. */
export const CATEGORIES: { value: video.VideoCategory; key: string; emoji: string }[] = [
  { value: VideoCategory.GAMING, key: "gaming", emoji: "🎮" },
  { value: VideoCategory.TALK, key: "talk", emoji: "💬" },
  { value: VideoCategory.MUSIC, key: "music", emoji: "🎵" },
  { value: VideoCategory.ENTERTAINMENT, key: "entertainment", emoji: "🎉" },
  { value: VideoCategory.DAILY_LIFE, key: "dailyLife", emoji: "☀️" },
  { value: VideoCategory.TRAVEL, key: "travel", emoji: "✈️" },
  { value: VideoCategory.FOOD, key: "food", emoji: "🍜" },
  { value: VideoCategory.SPORTS, key: "sports", emoji: "⚽" },
  { value: VideoCategory.EDUCATION, key: "education", emoji: "📚" },
  { value: VideoCategory.TECH, key: "tech", emoji: "💻" },
  { value: VideoCategory.ART, key: "art", emoji: "🎨" },
  { value: VideoCategory.PETS, key: "pets", emoji: "🐾" },
  { value: VideoCategory.NEWS, key: "news", emoji: "📰" },
  { value: VideoCategory.OTHER, key: "other", emoji: "✨" },
];

export function categoryOf(category: video.VideoCategory) {
  return CATEGORIES.find((entry) => entry.value === category);
}

export interface VisibilityOption {
  value: common.Visibility;
  /** `visibility.<key>` is its name, `visibility.<key>Hint` what it means. */
  key: string;
  icon: ComponentType<{ className?: string }>;
}

/** Who can watch a stream, from widest to narrowest. */
export const VISIBILITIES: VisibilityOption[] = [
  { value: Visibility.PUBLIC, key: "public", icon: Globe },
  { value: Visibility.NO_FEED, key: "noFeed", icon: EyeOff },
  { value: Visibility.UNLISTED, key: "unlisted", icon: Link2 },
  { value: Visibility.FRIENDS, key: "friends", icon: Users },
  { value: Visibility.PRIVATE, key: "private", icon: Lock },
];

export function visibilityOf(visibility: common.Visibility): VisibilityOption {
  return VISIBILITIES.find((option) => option.value === visibility) ?? VISIBILITIES[0]!;
}

/** Whether anyone with the link may watch (what a clip can be cut from). */
export function isOpenVisibility(visibility: common.Visibility): boolean {
  return (
    visibility === Visibility.PUBLIC ||
    visibility === Visibility.NO_FEED ||
    visibility === Visibility.UNLISTED
  );
}

/** `chatRole.<key>` names a badge next to a chat author; viewers have none. */
export const ROLE_KEYS: Partial<Record<live.ChatRole, string>> = {
  [live.ChatRole.SUBSCRIBER]: "subscriber",
  [live.ChatRole.MODERATOR]: "moderator",
  [live.ChatRole.OWNER]: "owner",
};

/** `endReason.<key>` says why a stream ended. */
export const END_REASON_KEYS: Partial<Record<live.EndReason, string>> = {
  [live.EndReason.STREAMER]: "streamer",
  [live.EndReason.ENCODER_GONE]: "encoderGone",
  [live.EndReason.TOO_LONG]: "tooLong",
  [live.EndReason.RAID]: "raid",
  [live.EndReason.REMOVED]: "removed",
};
