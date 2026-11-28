import { forgetUserQuest } from "@tokuly/core/quest/forget";
import type { ForgetUser } from "#/seams/forget-user";

/** After an unfriend or a block: their quest profile and the streak with them go. */
export default ((queryClient, userId) => {
  void forgetUserQuest(queryClient, userId);
}) satisfies ForgetUser;
