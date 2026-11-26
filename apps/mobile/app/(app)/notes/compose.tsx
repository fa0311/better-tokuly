import { useLocalSearchParams } from "expo-router";
import type { ComposeParams } from "#/features/notes/compose-href";
import { ComposeScreen } from "#/features/notes/compose-screen";

export default function Compose() {
  const params = useLocalSearchParams<ComposeParams & Record<string, string>>();
  return (
    <ComposeScreen
      replyTo={params.replyTo}
      quote={params.quote}
      draft={params.draft}
      text={params.text}
    />
  );
}
