import { useLocalSearchParams } from "expo-router";
import { BadgeScreen } from "#/features/quest/badge-screen";

export default function Badge() {
  const { key } = useLocalSearchParams<{ key: string }>();
  return <BadgeScreen key={key} badgeKey={key} />;
}
