import type { social } from "@tokuly/api";

/**
 * Whom the caller and the other side of a friend request both know: the
 * names the server sent (up to three) and how many more there are. `null`
 * when none (or when the other side's friend list is not the caller's to see).
 */
export function mutualFriendsOf(
  request: social.FriendRequest,
): { names: string[]; more: number } | null {
  if (request.mutualFriendCount <= 0) return null;
  const names = request.mutualFriends.map((user) => user.displayName || `@${user.handle}`);
  return { names, more: Math.max(0, request.mutualFriendCount - names.length) };
}
