/**
 * Where a short is being watched, as the address keeps it. Kept free of the
 * generated API code, so a route reading the address loads none of it.
 */

/** The list a swipe feed walks through: one of the three feeds, or the grid a short was opened from. */
export type FeedContext =
  | { kind: "forYou" }
  | { kind: "following" }
  | { kind: "friends" }
  | { kind: "channel"; key: string }
  /** A person's shorts across their channels (their profile); `key` is their user ID. */
  | { kind: "user"; key: string }
  | { kind: "hashtag"; key: string }
  | { kind: "sound"; key: string }
  | { kind: "remixes"; key: string }
  | { kind: "saved" }
  | { kind: "liked" }
  /** Opened by its link alone: the short, then the for-you feed. */
  | { kind: "link"; via?: "search" | "notification" };

export type FeedKind = FeedContext["kind"];

/** The grids a short can be opened from, as they appear in the address (`?list=tag&key=…`). */
export const LISTS = ["channel", "user", "tag", "sound", "remixes", "saved", "liked"] as const;
export type ListName = (typeof LISTS)[number];

export interface ShortSearch {
  list?: ListName;
  key?: string;
  via?: "search" | "notification";
}

export function validateShortSearch(search: Record<string, unknown>): ShortSearch {
  const list = LISTS.find((name) => name === search.list);
  const key = typeof search.key === "string" && search.key ? search.key : undefined;
  const via = search.via === "search" || search.via === "notification" ? search.via : undefined;
  if (list === "saved" || list === "liked") return { list };
  if (list && key) return { list, key };
  return via ? { via } : {};
}

export function contextOfSearch(search: ShortSearch): FeedContext {
  switch (search.list) {
    case "channel":
      return { kind: "channel", key: search.key ?? "" };
    case "user":
      return { kind: "user", key: search.key ?? "" };
    case "tag":
      return { kind: "hashtag", key: search.key ?? "" };
    case "sound":
      return { kind: "sound", key: search.key ?? "" };
    case "remixes":
      return { kind: "remixes", key: search.key ?? "" };
    case "saved":
      return { kind: "saved" };
    case "liked":
      return { kind: "liked" };
    default:
      return { kind: "link", via: search.via };
  }
}

/** The address search that opens a short inside the list it was tapped in. */
export function searchOfContext(context: FeedContext): ShortSearch {
  switch (context.kind) {
    case "channel":
      return { list: "channel", key: context.key };
    case "user":
      return { list: "user", key: context.key };
    case "hashtag":
      return { list: "tag", key: context.key };
    case "sound":
      return { list: "sound", key: context.key };
    case "remixes":
      return { list: "remixes", key: context.key };
    case "saved":
      return { list: "saved" };
    case "liked":
      return { list: "liked" };
    default:
      return {};
  }
}
