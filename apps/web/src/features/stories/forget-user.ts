import type { ForgetUser } from "#/seams/forget-user";
import { forgetUserStories } from "./api";
import { dropFromStoryViewer } from "./viewer-store";

/**
 * After an unfriend or a block: their reel leaves the story tray, and a
 * viewer that is playing their stories closes. (When the other side ends it,
 * the realtime connection says so and the same happens by itself.)
 */
export default ((queryClient, userId) => {
  forgetUserStories(queryClient, userId);
  dropFromStoryViewer(userId);
}) satisfies ForgetUser;
