import type { QueryClient } from "@tanstack/react-query";
import type { social } from "@tokuly/api";
import { FRIEND_PUSHES, friendKeys } from "@tokuly/core/friends";
import { reread } from "@tokuly/core/query";
import { forgetUser } from "#/seams/forget-user";

/**
 * What the realtime connection says about friends: a request came, went or
 * was accepted, and the lists are read again; a friendship ended (unfriended,
 * blocked, or the account was deleted, by either side) and every feature
 * forgets that person. Loaded with the first social event.
 */
export function applySocialEvent(queryClient: QueryClient, event: social.SocialEvent["event"]) {
  if (event.case === "friendRemovedUserId") forgetUser(queryClient, event.value);
  else if (FRIEND_PUSHES.has(event.case ?? "")) void reread(queryClient, [friendKeys.all]);
}
