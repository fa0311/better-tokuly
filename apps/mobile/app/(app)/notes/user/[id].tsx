import { useLocalSearchParams } from "expo-router";
import { UserNotesScreen } from "#/features/notes/user-notes-screen";

export default function UserNotes() {
  const { id } = useLocalSearchParams<{ id: string }>();
  return <UserNotesScreen key={id} userId={id} />;
}
