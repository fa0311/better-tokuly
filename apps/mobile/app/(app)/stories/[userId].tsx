import { useLocalSearchParams } from "expo-router";
import { StoryViewerScreen } from "#/features/stories/viewer-screen";

export default function Stories() {
  const { userId } = useLocalSearchParams<{ userId: string }>();
  return <StoryViewerScreen key={userId} userId={userId} />;
}
