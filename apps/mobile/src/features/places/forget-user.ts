import { forgetUserPlaces } from "@tokuly/core/places/forget";
import type { ForgetUser } from "#/seams/forget-user";

/** A friendship ended: their friends-only places leave what is loaded. */
export default ((queryClient, userId) => {
  void forgetUserPlaces(queryClient, userId);
}) satisfies ForgetUser;
