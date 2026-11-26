import { useLocalSearchParams } from "expo-router";
import { PackScreen } from "#/features/stamps/pack-screen";

export default function StampPack() {
  const { packId } = useLocalSearchParams<{ packId: string }>();
  return <PackScreen packId={packId ?? ""} />;
}
