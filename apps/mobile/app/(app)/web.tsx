import { useLocalSearchParams } from "expo-router";
import { WebScreen } from "#/features/web/web-screen";

/** `/web?path=/notes`: a screen of the web app, signed in as the account in use. */
export default function Web() {
  const { path } = useLocalSearchParams<{ path?: string }>();
  return <WebScreen path={path ?? "/"} />;
}
