import { forgetUserAsk } from "@tokuly/core/ask/forget";
import type { ForgetUser } from "#/seams/forget-user";

/** After an unfriend or a block: their box, answers and prompts leave what is loaded. */
export default ((queryClient, userId) => {
  void forgetUserAsk(queryClient, userId);
}) satisfies ForgetUser;
