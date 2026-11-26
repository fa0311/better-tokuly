import { common, video } from "@tokuly/api";
import { CircleDot, EyeOff, Globe, Link2, Lock, Star, UserRoundCheck, Users } from "lucide-react";
import type { ComponentType } from "react";

export { CATEGORIES, categoryKey } from "@tokuly/core/video";

const { TrafficSource } = video;
const { Visibility } = common;

export interface VisibilityOption {
  value: common.Visibility;
  /** `visibility.<key>` is its name, `visibility.<key>Hint` what it means. */
  key: string;
  icon: ComponentType<{ className?: string }>;
}

/** Who can see a video, from widest to narrowest (the social graph's audiences included). */
export const VIDEO_VISIBILITIES: VisibilityOption[] = [
  { value: Visibility.PUBLIC, key: "public", icon: Globe },
  { value: Visibility.NO_FEED, key: "noFeed", icon: EyeOff },
  { value: Visibility.UNLISTED, key: "unlisted", icon: Link2 },
  { value: Visibility.FOLLOWERS, key: "followers", icon: UserRoundCheck },
  { value: Visibility.FRIENDS, key: "friends", icon: Users },
  { value: Visibility.CLOSE_FRIENDS, key: "closeFriends", icon: Star },
  { value: Visibility.CIRCLE, key: "circle", icon: CircleDot },
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
