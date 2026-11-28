import type { QueryClient } from "@tanstack/query-core";
import type { social } from "@tokuly/api";
import { reread } from "../query";
import { followKeys, friendKeys } from "./keys";

/** The social pushes after which the friends lists and requests are read again. */
export const FRIEND_PUSHES: ReadonlySet<string> = new Set([
  "friendRequestReceived",
  "friendRequestCancelledId",
  "friendRequestAccepted",
]);

/**
 * A friendship ended (either side unfriended or blocked, or an account was
 * deleted): the person leaves every loaded friends list at once, what was
 * loaded about their follows goes, and everything social is read again — a
 * block also ends follows and requests both ways and takes them out of
 * circles on the server.
 */
export function forgetUserFriends(queryClient: QueryClient, userId: string): Promise<void> {
  queryClient.setQueriesData<social.ListFriendsResponse>({ queryKey: friendKeys.lists }, (data) => {
    if (!data?.friends.some((friend) => friend.user?.id === userId)) return data;
    const friends = data.friends.filter((friend) => friend.user?.id !== userId);
    return { ...data, friends, total: Math.max(0, data.total - 1) };
  });
  queryClient.removeQueries({ queryKey: followKeys.summary(userId) });
  queryClient.removeQueries({ queryKey: followKeys.userLists(userId) });
  // `friendKeys.all` is everything social: follows, follow suggestions and circles too.
  return reread(queryClient, [friendKeys.all]);
}
