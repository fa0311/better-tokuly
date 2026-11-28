import { forgetUserSpaces } from "@tokuly/core/spaces/forget";
import type { ForgetUser } from "#/seams/forget-user";

/** A friendship ended: rooms and hangouts shared with friends are read again. */
export default ((queryClient, userId) => {
  void forgetUserSpaces(queryClient, userId);
}) satisfies ForgetUser;
