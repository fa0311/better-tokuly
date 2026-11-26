import { useLocalSearchParams } from "expo-router";
import { HangoutScreen } from "#/features/spaces/hangout-screen";

export default function Hangout() {
  const { id } = useLocalSearchParams<{ id: string }>();
  return <HangoutScreen hangoutId={id ?? ""} />;
}
