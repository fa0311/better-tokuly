import { forgetUserVideo } from "@tokuly/core/video/forget";
import type { ForgetUser } from "#/seams/forget-user";

/** After an unfriend or a block: the feeds, channels, videos and playlists loaded are read again. */
export default ((queryClient, userId) => {
  void forgetUserVideo(queryClient, userId);
}) satisfies ForgetUser;
