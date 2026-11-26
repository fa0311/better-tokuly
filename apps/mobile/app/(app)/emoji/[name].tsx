import { useLocalSearchParams } from "expo-router";
import { DetailScreen } from "#/features/emoji/detail-screen";

export default function EmojiDetail() {
  const { name } = useLocalSearchParams<{ name: string }>();
  return <DetailScreen name={name ?? ""} />;
}
