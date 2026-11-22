import { createFileRoute } from "@tanstack/react-router";
import { PlaylistLibraryScreen } from "#/features/video/playlists/library-screen";

export const Route = createFileRoute("/_app/watch/playlists")({ component: PlaylistLibraryScreen });
