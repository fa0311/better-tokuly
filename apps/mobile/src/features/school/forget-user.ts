import { forgetUserSchool } from "@tokuly/core/school/forget";
import type { ForgetUser } from "#/seams/forget-user";

/** A friendship ended: their timetable and studying leave what is loaded. */
export default ((queryClient, userId) => {
  void forgetUserSchool(queryClient, userId);
}) satisfies ForgetUser;
