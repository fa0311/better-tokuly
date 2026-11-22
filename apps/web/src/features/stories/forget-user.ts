import type { QueryClient } from "@tanstack/react-query";
import { forgetUserNotes } from "#/features/notes";
import { forgetUserStories } from "./api";
import { dropFromStoryViewer } from "./viewer-store";

/**
 * Call this after the viewer unfriended or blocked someone. Their notes
 * leave every loaded timeline, their reel leaves the story tray, and a
 * viewer that is playing their stories closes. (When the other side ends it,
 * the realtime connection says so and the same happens by itself.)
 */
export function forgetUser(queryClient: QueryClient, userId: string): void {
  forgetUserNotes(queryClient, userId);
  forgetUserStories(queryClient, userId);
  dropFromStoryViewer(userId);
}
