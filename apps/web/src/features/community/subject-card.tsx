import { appPathOf } from "@tokuly/core/community";
import { GenericSubjectCard, type SubjectCardProps } from "#/features/engagement";

/**
 * A saved (or reacted-to) community, thread, reply or event: the generic
 * card, opening this app's address of the page instead of the one the
 * community domain publishes (`/c/…`, a video channel here).
 */
export function CommunitySubjectCard({ preview }: SubjectCardProps) {
  const path = appPathOf(preview.path);
  return <GenericSubjectCard preview={path ? { ...preview, path } : preview} />;
}
