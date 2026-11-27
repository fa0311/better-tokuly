import { useLocalSearchParams } from "expo-router";
import { CheckInScreen } from "#/features/places/check-in-screen";

export default function CheckIn() {
  const { id } = useLocalSearchParams<{ id: string }>();
  return <CheckInScreen placeId={id} />;
}
