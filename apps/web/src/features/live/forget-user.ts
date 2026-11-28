import { forgetUserLive } from "@tokuly/core/live/forget";
import type { ForgetUser } from "#/seams/forget-user";

/** After an unfriend or a block: the streams loaded, and whom the viewer blocked, are read again. */
export default ((queryClient, userId) => {
  void forgetUserLive(queryClient, userId);
}) satisfies ForgetUser;
