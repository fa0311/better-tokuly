import { forgetShorts } from "@tokuly/core/shorts";
import type { ForgetUser } from "#/seams/forget-user";

/**
 * A friendship ended: every loaded short goes and is read again where it is
 * on screen (whose a short is cannot be told from the cache). Swipe feeds
 * never go stale otherwise, so a friends-only short of someone who is no
 * longer a friend would go on playing.
 */
export default ((queryClient) => {
  void forgetShorts(queryClient);
}) satisfies ForgetUser;
