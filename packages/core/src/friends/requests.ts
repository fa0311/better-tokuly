import type { social } from "@tokuly/api";
import type { i18n as I18n } from "i18next";

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

/**
 * The line under a friend request: its message, and whom both know (with
 * names when the server sent them). `undefined` when there is neither.
 */
export function requestNote(i18n: I18n, request: social.FriendRequest): string | undefined {
  const mutual = mutualFriendsOf(request);
  const names = mutual?.names.join(i18n.t("friends:requests.nameSeparator"));
  const line = !mutual
    ? undefined
    : !names
      ? i18n.t("friends:requests.mutual", { count: request.mutualFriendCount })
      : mutual.more > 0
        ? i18n.t("friends:requests.mutualNamedMore", { names, count: mutual.more })
        : i18n.t("friends:requests.mutualNamed", { names });
  return [request.message, line].filter(Boolean).join(" · ") || undefined;
}
