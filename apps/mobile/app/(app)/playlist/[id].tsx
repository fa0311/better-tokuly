import { useLocalSearchParams } from "expo-router";
import { PlaylistScreen } from "#/features/video/playlist-screen";

export default function Playlist() {
  const { id } = useLocalSearchParams<{ id: string }>();
  return <PlaylistScreen key={id} playlistId={id} />;
}
