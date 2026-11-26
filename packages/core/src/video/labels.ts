import { video } from "@tokuly/api";

const { VideoCategory } = video;

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
