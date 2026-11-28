import type { QueryClient } from "@tanstack/query-core";
import { reread } from "../query";
import { videoKeys } from "./keys";

/**
 * A friendship ended. A video names its channel, not the person behind it, so
 * the feeds, channels, videos and playlists loaded are read again (not the
 * studio or the watch history: those are the viewer's own).
 */
export function forgetUserVideo(queryClient: QueryClient, _userId: string): Promise<void> {
  return reread(queryClient, [
    videoKeys.feeds,
    videoKeys.channels,
    videoKeys.videos,
    videoKeys.playlists,
  ]);
}
