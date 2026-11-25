import { useLocalSearchParams } from "expo-router";
import { ProfileScreen } from "#/features/profile/profile-screen";

export default function Profile() {
  const { handle } = useLocalSearchParams<{ handle: string }>();
  return <ProfileScreen handle={handle} />;
}
