import { useLocalSearchParams } from "expo-router";
import { SignUpScreen } from "#/features/session/sign-up-screen";

/** `?invite=<code>`: the sign-up counts as an intro of whoever invited. */
export default function SignUp() {
  const { invite } = useLocalSearchParams<{ invite?: string }>();
  return <SignUpScreen inviteCode={invite ?? ""} />;
}
