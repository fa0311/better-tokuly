import { useLocalSearchParams } from "expo-router";
import { WatchScreen } from "#/features/shorts/feed-screen";

export default function Watch() {
  const search = useLocalSearchParams<{
    list?: string;
    key?: string;
    via?: string;
    start?: string;
  }>();
  return <WatchScreen search={search} />;
}
