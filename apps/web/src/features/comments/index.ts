/**
 * What the comments feature shares with other features. Every kind of
 * content shows its comments with one component:
 *
 *   <CommentThread subject={{ type: common.SubjectType.NOTE, id: note.id }} />
 *
 * and, for videos, hands it the player so comments can point at moments:
 *
 *   <CommentThread
 *     subject={{ type: common.SubjectType.VIDEO, id: video.id }}
 *     subjectAuthor={{ channel: video.channel }}        // creator badge
 *     focusCommentId={search.comment}                   // `?comment=<id>` from a notification
 *     video={{ currentTime: () => player.currentTime, seekTo: (seconds) => player.seek(seconds) }}
 *   />
 *
 * `useHeldCount()` is the number of comments waiting for the signed-in
 * person's review (for a badge next to a link to `/settings/comments`).
 */
export { useHeldCount } from "./api";
export { CommentThread, type CommentThreadProps } from "./thread";
export type { CommentVideoBridge } from "./thread-context";
