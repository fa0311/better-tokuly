import { useLocalSearchParams } from "expo-router";
import { NoteScreen } from "#/features/notes/note-screen";

export default function Note() {
  const { id } = useLocalSearchParams<{ id: string }>();
  return <NoteScreen key={id} noteId={id} />;
}
