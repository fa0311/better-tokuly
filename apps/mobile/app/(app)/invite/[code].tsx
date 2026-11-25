import { useLocalSearchParams } from "expo-router";
import { InviteScreen } from "#/features/friends/invite-screen";

/** `tokuly://invite/<code>`: what an invite link opens in the app. */
export default function Invite() {
  const { code } = useLocalSearchParams<{ code: string }>();
  return <InviteScreen key={code} code={code} />;
}
