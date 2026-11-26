import { useLocalSearchParams } from "expo-router";
import { ComposeThreadScreen } from "#/features/community/compose-screen";

export default function NewThread() {
  const { handle, channel } = useLocalSearchParams<{ handle: string; channel?: string }>();
  return <ComposeThreadScreen handle={handle} channel={channel} />;
}
