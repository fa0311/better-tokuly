import { common, geo } from "@tokuly/api";

/** How each kind of place looks: its default emoji, the colour of its bubble and its name in the strings (`labels.<key>`). */
export const LABELS = {
  [geo.PlaceLabel.HOME]: { key: "home", emoji: "🏠", tint: "bg-sun" },
  [geo.PlaceLabel.SCHOOL]: { key: "school", emoji: "🏫", tint: "bg-sky" },
  [geo.PlaceLabel.WORK]: { key: "work", emoji: "💼", tint: "bg-mint" },
  [geo.PlaceLabel.CUSTOM]: { key: "custom", emoji: "📍", tint: "bg-pink" },
} as const;

export type LabelKey = (typeof LABELS)[keyof typeof LABELS]["key"];

export const labelMeta = (label: geo.PlaceLabel) =>
  LABELS[label as keyof typeof LABELS] ?? LABELS[geo.PlaceLabel.CUSTOM];

export function labelFromKey(key: string | undefined): geo.PlaceLabel {
  const found = Object.entries(LABELS).find(([, meta]) => meta.key === key);
  return found ? (Number(found[0]) as geo.PlaceLabel) : geo.PlaceLabel.CUSTOM;
}

/** The emoji a place is drawn with: its own, else the one of its kind. */
export const placeEmoji = (place: Pick<geo.Place, "emoji" | "label">): string =>
  place.emoji || labelMeta(place.label).emoji;

/** Emoji offered first when naming a place. */
export const EMOJI_CHOICES = [
  "📍",
  "🏠",
  "🏫",
  "💼",
  "☕",
  "🍜",
  "🍔",
  "🛍️",
  "🎮",
  "🎤",
  "⚽",
  "🌳",
  "🚉",
  "📚",
  "💖",
  "⭐",
];

/** Categories offered for a place. The API takes any short text; these are the ones the app names. */
export const CATEGORIES = [
  "cafe",
  "food",
  "shop",
  "park",
  "station",
  "school",
  "fun",
  "sports",
  "other",
] as const;

/** Who a place can be shown to (`visibility.<key>` in the strings). */
export const VISIBILITIES = [
  { value: common.Visibility.PRIVATE, key: "private" },
  { value: common.Visibility.FRIENDS, key: "friends" },
  { value: common.Visibility.PUBLIC, key: "public" },
] as const;

export const visibilityKey = (visibility: common.Visibility): string =>
  VISIBILITIES.find((entry) => entry.value === visibility)?.key ?? "private";
