import { forgetUserAsk } from "@tokuly/core/ask/forget";
import type { ForgetUser } from "#/seams/forget-user";

/** A friendship ended: their box, answers and prompts leave what is loaded. */
export default ((queryClient, userId) => {
  void forgetUserAsk(queryClient, userId);
}) satisfies ForgetUser;
