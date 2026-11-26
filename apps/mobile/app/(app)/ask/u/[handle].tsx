import { useLocalSearchParams } from "expo-router";
import { BoxScreen } from "#/features/ask/box-screen";

export default function AskBox() {
  const { handle } = useLocalSearchParams<{ handle: string }>();
  return <BoxScreen handle={handle ?? ""} />;
}
