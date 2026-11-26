import { useLocalSearchParams } from "expo-router";
import { ClipScreen } from "#/features/notes/clips-screen";

export default function Clip() {
  const { id } = useLocalSearchParams<{ id: string }>();
  return <ClipScreen key={id} clipId={id} />;
}
