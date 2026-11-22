import { GenericSubjectCard, type SubjectCardProps } from "#/features/engagement";
import { Skeleton } from "#/components/ui/page";
import { useNote } from "./api";
import { NoteCard } from "./note-card";

/** A saved or reacted-to note on `/bookmarks`: the note itself, or the generic row if it cannot be loaded. */
export function NoteSubjectCard({ preview }: SubjectCardProps) {
  const note = useNote(preview.subject?.id ?? "");
  if (note.isPending) return <Skeleton className="h-28" />;
  if (!note.data) return <GenericSubjectCard preview={preview} />;
  return <NoteCard note={note.data} />;
}
