import { forgetUserPlans } from "@tokuly/core/plans/forget";
import type { ForgetUser } from "#/seams/forget-user";

/** A friendship ended: what friends share in plans is read again. */
export default ((queryClient, userId) => {
  void forgetUserPlans(queryClient, userId);
}) satisfies ForgetUser;
