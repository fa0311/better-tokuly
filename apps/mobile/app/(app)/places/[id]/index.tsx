import { useLocalSearchParams } from "expo-router";
import { PlaceScreen } from "#/features/places/place-screen";

export default function Place() {
  const { id } = useLocalSearchParams<{ id: string }>();
  return <PlaceScreen key={id} placeId={id} />;
}
