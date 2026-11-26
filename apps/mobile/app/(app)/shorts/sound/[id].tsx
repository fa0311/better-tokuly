import { useLocalSearchParams } from "expo-router";
import { SoundScreen } from "#/features/shorts/sound-screen";

export default function Sound() {
  const { id } = useLocalSearchParams<{ id: string }>();
  return <SoundScreen key={id} soundId={id} />;
}
