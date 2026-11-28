import { forgetUserMoments } from "@tokuly/core/moments";
import type { ForgetUser } from "#/seams/forget-user";

/** After an unfriend or a block: their moments leave the friends' list and the map. */
export default ((queryClient, userId) =>
  forgetUserMoments(queryClient, userId)) satisfies ForgetUser;
