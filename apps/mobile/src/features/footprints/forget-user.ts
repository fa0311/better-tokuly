import { forgetUserFootprints } from "@tokuly/core/footprints/forget";
import type { ForgetUser } from "#/seams/forget-user";

/** A friendship ended: their footprints go, and the leaderboards are read again. */
export default ((queryClient, userId) => {
  void forgetUserFootprints(queryClient, userId);
}) satisfies ForgetUser;
