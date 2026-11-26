import { useLocalSearchParams } from "expo-router";
import { TagScreen } from "#/features/notes/tag-screen";

export default function Tag() {
  const { tag } = useLocalSearchParams<{ tag: string }>();
  return <TagScreen key={tag} tag={tag} />;
}
