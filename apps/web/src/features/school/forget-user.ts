import { forgetUserSchool } from "@tokuly/core/school/forget";
import type { ForgetUser } from "#/seams/forget-user";

/** After an unfriend or a block: their timetable and studying leave what is loaded. */
export default ((queryClient, userId) => {
  void forgetUserSchool(queryClient, userId);
}) satisfies ForgetUser;
