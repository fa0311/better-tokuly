import { forgetUserPlaces } from "@tokuly/core/places/forget";
import type { ForgetUser } from "#/seams/forget-user";

/** After an unfriend or a block: their friends-only places leave what is loaded. */
export default ((queryClient, userId) => {
  void forgetUserPlaces(queryClient, userId);
}) satisfies ForgetUser;
