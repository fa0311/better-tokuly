import { useLocalSearchParams } from "expo-router";
import { StoreScreen } from "#/features/stamps/store-screen";

export default function StampStore() {
  const { q, tag } = useLocalSearchParams<{ q?: string; tag?: string }>();
  return <StoreScreen query={q} tag={tag} />;
}
