import { Stack, useLocalSearchParams } from "expo-router";
import { HashtagScreen } from "#/features/shorts/hashtag-screen";

export default function Hashtag() {
  const { tag } = useLocalSearchParams<{ tag: string }>();
  return (
    <>
      <Stack.Screen options={{ title: `#${tag}` }} />
      <HashtagScreen key={tag} tag={tag} />
    </>
  );
}
