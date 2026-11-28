import { forgetUserSpaces } from "@tokuly/core/spaces/forget";
import type { ForgetUser } from "#/seams/forget-user";

/** After an unfriend or a block: rooms and hangouts shared with friends are read again. */
export default ((queryClient, userId) => {
  void forgetUserSpaces(queryClient, userId);
}) satisfies ForgetUser;
