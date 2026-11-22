import { createFileRoute } from "@tanstack/react-router";
import { PlaylistScreen } from "#/features/video/playlists/playlist-screen";

/** `/playlist/<short id>`; `WL` and `LL` are the viewer's watch-later list and liked videos. */
export const Route = createFileRoute("/_app/playlist/$playlistId")({ component: PlaylistPage });

function PlaylistPage() {
  const { playlistId } = Route.useParams();
  return <PlaylistScreen key={playlistId} playlistId={playlistId} />;
}
