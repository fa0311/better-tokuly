/**
 * The communities' categories by their name in an address
 * (`/communities?category=club`), in the order the directory lists them.
 * Kept free of the generated API code, so a route reading the address loads none of it.
 */
export const CATEGORY_KEYS = [
  "school",
  "club",
  "game",
  "oshi",
  "local",
  "study",
  "tech",
  "other",
] as const;
export type CategoryKey = (typeof CATEGORY_KEYS)[number];
