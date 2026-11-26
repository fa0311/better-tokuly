import { useLocalSearchParams } from "expo-router";
import { WatchScreen } from "#/features/video/watch-screen";

export default function Watch() {
  const { videoId, t, list, via } = useLocalSearchParams<{
    videoId: string;
    t?: string;
    list?: string;
    via?: string;
  }>();
  return <WatchScreen key={videoId} videoId={videoId} params={{ t, list, via }} />;
}
