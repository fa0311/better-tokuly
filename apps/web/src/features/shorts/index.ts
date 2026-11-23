/**
 * What the shorts feature shares with other features.
 *
 * Showing a short anywhere — a search result, a chat message, a note, a
 * profile — is a `ShortTile`: its cover in 9:16 with the view count, leading
 * to the swipe feed opened on it.
 *
 *   <ShortGrid>
 *     {shorts.map((short) => <ShortTile key={short.id} short={short} via="search" />)}
 *   </ShortGrid>
 *
 * `context` makes the swipe feed walk through the list the tile stands in
 * (`{ kind: "channel", key: handle }`, `{ kind: "hashtag", key: tag }`,
 * `{ kind: "sound", key: soundId }`); `<ShortList context … />` is such a
 * list with its loading, paging and empty state.
 *
 * `shortSubject(short)` is the short for the shared controls (reactions,
 * comments, reports, "send in chat"). Links: `/shorts/<short id>` plays a
 * short, `/shorts/tags/<tag>` is a hashtag, `/sound/<id>` a sound,
 * `/shorts/new?clip=<clip id>` starts a short from a clip.
 */

export { Caption } from "./caption";
export { type FeedContext, shortSubject } from "./model";
export {
  ShortGrid,
  ShortGridSkeleton,
  ShortList,
  ShortTile,
  type ShortTileProps,
} from "./short-tile";
