import { common, video } from "@tokuly/api";
import { EyeOff, Globe, Link2, Lock, Users } from "lucide-react";
import type { ComponentType } from "react";

const { VideoCategory, TrafficSource } = video;
const { Visibility } = common;

/** The genres, in the order the chips and the studio's menu list them. `key` is `category.<key>` in the strings. */
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

/** The string key of a genre's name; `undefined` for "no genre". */
export function categoryKey(category: video.VideoCategory): string | undefined {
  const found = CATEGORIES.find((entry) => entry.value === category);
  return found && `category.${found.key}`;
}

export interface VisibilityOption {
  value: common.Visibility;
  /** `visibility.<key>` is its name, `visibility.<key>Hint` what it means. */
  key: string;
  icon: ComponentType<{ className?: string }>;
}

/** Who can see a video, from widest to narrowest. */
export const VIDEO_VISIBILITIES: VisibilityOption[] = [
  { value: Visibility.PUBLIC, key: "public", icon: Globe },
  { value: Visibility.NO_FEED, key: "noFeed", icon: EyeOff },
  { value: Visibility.UNLISTED, key: "unlisted", icon: Link2 },
  { value: Visibility.FRIENDS, key: "friends", icon: Users },
  { value: Visibility.PRIVATE, key: "private", icon: Lock },
];

/** Playlists know three audiences. */
export const PLAYLIST_VISIBILITIES: VisibilityOption[] = VIDEO_VISIBILITIES.filter(
  (option) =>
    option.value === Visibility.PUBLIC ||
    option.value === Visibility.UNLISTED ||
    option.value === Visibility.PRIVATE,
);

export function visibilityOption(visibility: common.Visibility): VisibilityOption {
  return (
    VIDEO_VISIBILITIES.find((option) => option.value === visibility) ??
    (VIDEO_VISIBILITIES.at(-1) as VisibilityOption)
  );
}

/** `traffic.<key>` names where viewers came from, in the studio's analytics. */
export const TRAFFIC_SOURCE_KEYS: Record<number, string> = {
  [TrafficSource.UNSPECIFIED]: "other",
  [TrafficSource.DIRECT]: "direct",
  [TrafficSource.HOME]: "home",
  [TrafficSource.SUBSCRIPTIONS]: "subscriptions",
  [TrafficSource.TRENDING]: "trending",
  [TrafficSource.RELATED]: "related",
  [TrafficSource.SEARCH]: "search",
  [TrafficSource.CHANNEL]: "channel",
  [TrafficSource.PLAYLIST]: "playlist",
  [TrafficSource.NOTIFICATION]: "notification",
  [TrafficSource.FRIENDS]: "friends",
  [TrafficSource.WATCH_PARTY]: "watchParty",
  [TrafficSource.EXTERNAL]: "external",
  [TrafficSource.HISTORY]: "history",
};
