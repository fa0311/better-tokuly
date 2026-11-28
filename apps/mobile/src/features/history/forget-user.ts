import { forgetUserHistory } from "@tokuly/core/history/forget";
import type { ForgetUser } from "#/seams/forget-user";

/** A friendship ended: the history is read again, without them among the friends met. */
export default ((queryClient, userId) => {
  void forgetUserHistory(queryClient, userId);
}) satisfies ForgetUser;
