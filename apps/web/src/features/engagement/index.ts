/**
 * What the engagement feature shares with every content feature: reactions,
 * saving, sharing and reporting for anything that has a `SubjectRef`.
 *
 *   const subject = { type: common.SubjectType.VIDEO, id: video.id };   // UUID or short ID
 *
 *   <ReactionBar subject={subject} />                 // like + emoji reactions (Unicode and custom), live counts and pops, who reacted
 *   <ReactionEmoji emoji={reaction.emoji} custom={…} />   // one emoji of a reaction, custom ones as their picture
 *   <BookmarkButton subject={subject} showCount />    // save; the collection chooser when already saved
 *   <ShareButton subject={subject} title={video.title} />   // send in chat, copy the link, the share sheet
 *   const more = useSubjectMenu(subject, { title, label });   // "send in chat", "copy link", "report" for a menu of your own
 *   const toChat = useShareToChat(subject, title);     // the same "send in chat" for a menu of your own
 *   <ReportDialog subject={subject} open={reporting} onOpenChange={setReporting} label={video.title} />
 *
 *   const { data } = useEngagement(subject);          // counts and the viewer's own state; `null` = not visible
 *   useSubjectLive(subject);                          // keep them live while this component is on screen
 *
 * Lists need no batching code: every row asks for its own subject and the
 * requests of one moment travel together (`prefetchEngagement` warms a page
 * ahead of rendering). Saved and reacted-to subjects are drawn on
 * `/bookmarks` by the card each feature registers with `registerSubjectCard`.
 */
export {
  loadEngagement,
  prefetchEngagement,
  useEngagement,
  useReact,
  useShareStats,
  useUnreact,
} from "./api";
export { BookmarkButton, type BookmarkButtonProps } from "./bookmark-button";
export { useBurst } from "./burst";
export { openCollectionChooser } from "./collection-chooser";
export { EmojiPicker, type EmojiPickerProps } from "./emoji-picker";
export { useSubjectLive, useSubjectTopic } from "./live";
export {
  compactCount,
  type Engagement,
  emojiReaction,
  LIKE,
  type ReactionPicture,
  type ReactionRules,
  type Subject,
  subjectTopic,
  type ViewerReaction,
} from "./model";
export { useOpenPath } from "./open-path";
export {
  AddReactionButton,
  LikeButton,
  ReactionBar,
  type ReactionBarProps,
  ReactionChips,
  ReactorsDialog,
} from "./reaction-bar";
export { ReactionEmoji, type ReactionEmojiProps } from "./reaction-emoji";
export { ReportButton, ReportDialog, type ReportDialogProps } from "./report-dialog";
export { ReportsList } from "./reports-list";
export {
  copyText,
  ShareButton,
  type ShareButtonProps,
  useCopySubjectLink,
  useShareToChat,
} from "./share-button";
export { GenericSubjectCard, registerSubjectCard, type SubjectCardProps } from "./subject-card";
export { type SubjectMenuOptions, SubjectMoreButton, useSubjectMenu } from "./subject-menu";
