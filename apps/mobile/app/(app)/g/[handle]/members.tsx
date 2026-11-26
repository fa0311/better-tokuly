import { useLocalSearchParams } from "expo-router";
import { MembersScreen } from "#/features/community/members-screen";

export default function Members() {
  const { handle } = useLocalSearchParams<{ handle: string }>();
  return <MembersScreen key={handle} handle={handle} />;
}
