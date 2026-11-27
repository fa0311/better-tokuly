import { shortKeys } from "@tokuly/core/shorts/keys";
import type { ForgetUser } from "#/seams/forget-user";

/**
 * After an unfriend or a block. A short names its channel, not the person
 * behind it, so there is no telling which cached shorts were theirs: every
 * loaded feed, grid and short is dropped (and read again where it is on
 * screen). Feeds keep their lists for good otherwise, so without this a
 * blocked person's shorts would go on playing.
 */
export default ((queryClient) => {
  void queryClient.resetQueries({ queryKey: shortKeys.all });
}) satisfies ForgetUser;
