import { forgetUserFriends } from "@tokuly/core/friends";
import type { ForgetUser } from "#/seams/forget-user";

/**
 * After an unfriend or a block: they leave the friends list, what was loaded
 * about their follows goes, and the lists they may sit in (requests, follows,
 * suggestions, circles) load again.
 */
export default ((queryClient, userId) => {
  void forgetUserFriends(queryClient, userId);
}) satisfies ForgetUser;
