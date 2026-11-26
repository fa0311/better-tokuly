import { useLocalSearchParams } from "expo-router";
import { CommunityScreen } from "#/features/community/community-screen";

export default function Community() {
  const { handle } = useLocalSearchParams<{ handle: string }>();
  return <CommunityScreen key={handle} handle={handle} />;
}
