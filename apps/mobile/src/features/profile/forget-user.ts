import { forgetUserProfile } from "@tokuly/core/profile/forget";
import type { ForgetUser } from "#/seams/forget-user";

/** A friendship ended: profiles are read again. */
export default ((queryClient, userId) => {
  void forgetUserProfile(queryClient, userId);
}) satisfies ForgetUser;
