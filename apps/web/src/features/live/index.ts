/**
 * What the live feature shares with other features.
 *
 * Showing a stream anywhere — a search result, a friend's activity, a chat
 * message, a profile — is a `StreamCard` (it leads to `/live/<id>` and shows
 * whether the stream is on air, scheduled with a countdown, or over):
 *
 *   <StreamCard stream={summary} layout="row" />
 *
 * `LiveMark` is the red "LIVE" mark for anything that is on air right now.
 * A clip is a `ClipCard` (it leads to `/clip/<id>`); `ClipStrip` is a row of
 * the newest clips cut from a stream, a video or a channel, or cut by a
 * person, and renders nothing when there are none:
 *
 *   <ClipStrip kind="video" id={video.id} />
 *
 * "Make a clip of this video" is a link to the clip editor:
 *
 *   <Link to="/live/clips/new" search={{ video: video.shortId }}>クリップを作る</Link>
 *
 * `useFriendStreams()` is the streams of the signed-in person's friends
 * (friends-only ones included), on air first.
 */
export { useFriendStreams } from "./api";
export { ClipCard, ClipStrip } from "./clips/clip-list";
export { LiveMark, StreamCard, StreamCardSkeleton, StreamGrid } from "./parts";
