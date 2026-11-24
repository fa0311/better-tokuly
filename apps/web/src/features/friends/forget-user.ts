import type { ForgetUser } from "#/seams/forget-user";
import { circleKeys, followKeys } from "./follow-data";

/**
 * After an unfriend or a block: what was loaded about that person's follows
 * goes, and the lists they may sit in (follows, requests, suggestions,
 * circles) load again — a block ends follows both ways and takes them out of
 * circles on the server.
 */
export default ((queryClient, userId) => {
  queryClient.removeQueries({ queryKey: followKeys.summary(userId) });
  queryClient.removeQueries({ queryKey: followKeys.userLists(userId) });
  void queryClient.invalidateQueries({ queryKey: followKeys.all });
  void queryClient.invalidateQueries({ queryKey: followKeys.suggestions });
  void queryClient.invalidateQueries({ queryKey: circleKeys.all });
}) satisfies ForgetUser;
