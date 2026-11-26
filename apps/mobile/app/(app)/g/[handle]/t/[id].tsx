import { useLocalSearchParams } from "expo-router";
import { ThreadScreen } from "#/features/community/thread-screen";

export default function Thread() {
  const { handle, id } = useLocalSearchParams<{ handle: string; id: string }>();
  return <ThreadScreen key={id} handle={handle} threadId={id} />;
}
