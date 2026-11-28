import { liveKeys } from "@tokuly/core/live/keys";
import { reread } from "@tokuly/core/query";
import type { ForgetUser } from "#/seams/forget-user";

/**
 * After an unfriend or a block (a block of a friend ends the friendship
 * too): whom the viewer blocked is read again, so an open stream's chat
 * leaves out their lines.
 */
export default ((queryClient) => {
  void reread(queryClient, [liveKeys.blocked]);
}) satisfies ForgetUser;
