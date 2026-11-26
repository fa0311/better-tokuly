import { useLocalSearchParams } from "expo-router";
import { AntennaScreen } from "#/features/notes/list-screen";

export default function Antenna() {
  const { id } = useLocalSearchParams<{ id: string }>();
  return <AntennaScreen key={id} antennaId={id} />;
}
