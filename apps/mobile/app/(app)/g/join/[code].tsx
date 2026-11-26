import { useLocalSearchParams } from "expo-router";
import { InviteScreen } from "#/features/community/invite-screen";

export default function CommunityInvite() {
  const { code } = useLocalSearchParams<{ code: string }>();
  return <InviteScreen key={code} code={code} />;
}
