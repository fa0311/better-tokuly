import { liveKeys } from "@tokuly/core/live";
import { reread } from "@tokuly/core/query";
import type { ForgetUser } from "#/seams/forget-user";

/**
 * A friendship ended, perhaps through a block on this account: whom the
 * viewer blocked is read again, so an open stream's chat leaves out their lines.
 */
export default ((queryClient) => {
  void reread(queryClient, [liveKeys.blocked]);
}) satisfies ForgetUser;
