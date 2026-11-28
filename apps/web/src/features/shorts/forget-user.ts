import { forgetShorts } from "@tokuly/core/shorts";
import type { ForgetUser } from "#/seams/forget-user";

/**
 * After an unfriend or a block: every loaded short goes and is read again
 * where it is on screen (whose a short is cannot be told from the cache).
 */
export default ((queryClient) => {
  void forgetShorts(queryClient);
}) satisfies ForgetUser;
