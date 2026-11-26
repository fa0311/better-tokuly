/**
 * What the notes feature shares with other features.
 *
 *   <NoteCard note={note} />                        // a note anywhere: text, media, counters, every action
 *   openCompose({ quote: note })                    // the composer: a new note, a reply, a quote
 *   <ProfileNotes userId={user.id} isSelf />        // the notes part of a profile
 *
 * and, for features that show pictures and videos of their own: the upload
 * tray (`useUploads`, `UploadTile`), the full-screen `ImageViewer`, and the
 * HLS-aware `useVideoSource`.
 */

export { charCount } from "@tokuly/core/notes";
export { useNote } from "./api";
export { type ComposeContext, openCompose } from "./compose-store";
export {
  attachmentImage,
  ImageViewer,
  imageUrl,
  isVideo,
  useOnScreen,
  useVideoSource,
} from "./media";
export { NoteCard, type NoteCardProps } from "./note-card";
export { ProfileNotes, type ProfileNotesProps } from "./profile-notes";
export { NoteText } from "./rich-text";
export { type Upload, type UploadRules, UploadTile, useUploads } from "./uploads";
