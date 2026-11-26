import { useLocalSearchParams } from "expo-router";
import { ChannelScreen } from "#/features/video/channel-screen";

export default function Channel() {
  const { handle } = useLocalSearchParams<{ handle: string }>();
  return <ChannelScreen key={handle} handle={handle} />;
}
