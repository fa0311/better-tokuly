import { useLocalSearchParams } from "expo-router";
import { WatchLiveScreen } from "#/features/live/watch-screen";

export default function WatchLive() {
  const { id } = useLocalSearchParams<{ id: string }>();
  return <WatchLiveScreen streamId={id ?? ""} />;
}
