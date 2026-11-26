import { useLocalSearchParams } from "expo-router";
import { ListScreen } from "#/features/notes/list-screen";

export default function List() {
  const { id } = useLocalSearchParams<{ id: string }>();
  return <ListScreen key={id} listId={id} />;
}
